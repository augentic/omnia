//! The `omnia:plugins/loader` load path.

use std::collections::HashMap;
use std::future::Future;
use std::path::Path;
use std::sync::Arc;

use anyhow::{Context as _, bail, ensure};
use cap_fs_ext::MetadataExt as _;
use cap_std::fs::Dir;
use omnia_core::{AdmitError, Digest, GuestId, MountRegistry, Runtime, Source, SourceSpec};

use crate::admission::{Admission, Registration};
use crate::error::LoadError;
use crate::source::{Location, RegistrySource};

/// Host-side `omnia:plugins/loader.load` — embedder sugar over the runtime's
/// installed [`Plugins`] extension.
pub trait PluginLoader {
    /// Ensure the guest `from` names is active and return its handle, held
    /// to `pin` when one is given. Idempotent on (name, digest).
    ///
    /// # Errors
    ///
    /// `refused` on a location the deployment's grant does not serve, a
    /// digest mismatch, bytes that are not a loadable component, or a name
    /// active under other bytes; `unavailable` when the source could not
    /// produce the bytes; `internal` on registration failure.
    fn load(
        &self, from: Location, pin: Option<Digest>,
    ) -> impl Future<Output = Result<Plugin, LoadError>> + Send;
}

impl<B: Clone + Send + Sync + 'static> PluginLoader for Runtime<B> {
    fn load(
        &self, from: Location, pin: Option<Digest>,
    ) -> impl Future<Output = Result<Plugin, LoadError>> + Send {
        let plugins = self.extensions().get::<Plugins>();
        async move {
            match plugins {
                Some(plugins) => plugins.load(from, pin).await,
                None => Err(LoadError::no_plugins(&from)),
            }
        }
    }
}

/// The deployment's grant over the runtime's admission seam: its on-demand
/// guest table, its read-only mounts, and the registry its packages are
/// fetched from.
///
/// The grant is fixed at install. `load` resolves a location inside it — a
/// declared name to its entry, a path to the read-only mount beneath which
/// it lies, a package to the registry the deployment routes it to — and
/// admits the bytes it finds there. Nothing a caller passes widens it.
pub struct Plugins {
    guests: HashMap<GuestId, Source>,
    registry: Arc<dyn RegistrySource>,
    mounts: Vec<Root>,
    admission: Box<dyn Admission>,
}

// One of the deployment's mounts as a path load sees it: the handle it
// reads through, and whether the guest can write beneath it.
#[derive(Debug)]
struct Root {
    name: String,
    dir: Arc<Dir>,
    writable: bool,
}

impl Plugins {
    /// Install the loader capability on `runtime`: `guests` is every source
    /// the deployment declares for on-demand loading, `registry` fetches the
    /// [`SourceSpec::Package`] sources, and the runtime's mounts are the
    /// roots a path load resolves against — the read-only ones as code
    /// roots, the writable ones as refusals. Mount directories are told
    /// apart by identity, not path: two mount points of one directory are
    /// one directory here.
    ///
    /// # Errors
    ///
    /// Returns an error if an on-demand name repeats or is already registered
    /// at boot, a writable mount and a read-only mount share or nest their
    /// directories, or the capability is already installed.
    pub fn install<B>(
        runtime: &Runtime<B>, guests: impl IntoIterator<Item = Source>,
        registry: Arc<dyn RegistrySource>,
    ) -> anyhow::Result<()>
    where
        B: Clone + Send + Sync + 'static,
    {
        let mut table = HashMap::new();
        for source in guests {
            let id = source.id().clone();
            ensure!(
                runtime.registry().get(&id).is_none(),
                "on-demand guest `{id}` is already registered at boot"
            );
            ensure!(
                table.insert(id.clone(), source).is_none(),
                "on-demand guest `{id}` is declared twice"
            );
        }

        let plugins = Self {
            guests: table,
            registry,
            mounts: roots(runtime.mounts())?,
            admission: Box::new(runtime.downgrade()),
        };
        ensure!(
            runtime.extensions().insert(plugins),
            "the plugins capability installs exactly once per runtime"
        );
        Ok(())
    }

    /// Ensure the guest `from` names is active and return its handle: its
    /// bytes acquired inside the deployment's grant, held to `pin` when one
    /// is given, verified, and admitted through the runtime's admission seam
    /// under the name the location derives. A name already active under the
    /// same bytes is attested. Idempotent on (name, digest).
    ///
    /// # Errors
    ///
    /// `refused` on an undeclared name, a declared name with a pin, a path
    /// or package deriving a name the deployment declares, a path beneath no
    /// read-only mount, a package nothing routes, a digest mismatch, a
    /// pre-compiled artifact where raw wasm alone is admitted, bytes that
    /// are not a loadable component, or a name active under other bytes;
    /// `unavailable` when the source could not produce the bytes; `internal`
    /// on registration failure.
    pub async fn load(&self, from: Location, pin: Option<Digest>) -> Result<Plugin, LoadError> {
        let id = from.id();

        // resolve the location to the source its bytes must satisfy
        let (mut source, endpoint) = match &from {
            Location::Declared(name) => {
                if pin.is_some() {
                    return Err(LoadError::Refused(format!(
                        "`{name}` is declared by the deployment, which pins it; the load takes \
                         no digest"
                    )));
                }
                if let Registration::Active(digest) = self.admission.registration(&id)? {
                    return Ok(Plugin { id, digest });
                }
                let Some(source) = self.guests.get(&id) else {
                    return Err(LoadError::Refused(format!(
                        "no guest `{name}` is declared by this deployment"
                    )));
                };
                (source.clone(), None)
            }
            // A declared name is bound by its entry alone: a path or package
            // deriving it would seat the caller's bytes where the deployment's
            // belong, for the declared load to attest later.
            Location::Path(_) | Location::Registry { .. } if self.guests.contains_key(&id) => {
                return Err(LoadError::Refused(format!(
                    "`{from}` would register as `{id}`, a guest this deployment declares; load \
                     it as the declared guest `{id}`, whose source and pin are the deployment's"
                )));
            }
            Location::Path(path) => {
                let bytes = self.read(path).await?;
                (Source::new(id.clone(), bytes).wasm_only(), None)
            }
            Location::Registry { package, endpoint } => {
                let source = Source::new(id.clone(), SourceSpec::package(package)).wasm_only();
                (source, endpoint.as_deref())
            }
        };
        if let Some(pin) = pin {
            source = source.pinned(pin);
        }

        // acquire the bytes and hold them to what the source declares
        let bytes = match source.spec() {
            SourceSpec::Package(package) => self.registry.acquire(package, endpoint).await?,
            SourceSpec::Path(_) | SourceSpec::Bytes(_) => {
                source.read().await.map_err(|error| LoadError::Unavailable(format!("{error:#}")))?
            }
        };
        let digest =
            source.verified(&bytes).map_err(|error| LoadError::Refused(format!("{error:#}")))?;

        // admit them, or attest the registration that got there first
        match self.admission.admit(id.clone(), bytes, digest).await {
            Ok(()) => {
                tracing::debug!(%id, %digest, "guest loaded");
                Ok(Plugin { id, digest })
            }
            // Another load registered this name first — a racing load of the
            // same location, or a location naming an active guest. The same
            // bytes attest; other bytes never re-bind an active name.
            Err(AdmitError::AlreadyRegistered(_)) => match self.admission.registration(&id)? {
                Registration::Active(recorded) if recorded == digest => Ok(Plugin { id, digest }),
                Registration::Active(recorded) => Err(LoadError::Refused(format!(
                    "`{from}` would register as `{id}`, which is active under other bytes \
                     ({recorded}, not {digest})"
                ))),
                Registration::Absent => Err(LoadError::Internal(format!(
                    "`{id}` was admitted by a racing load and deregistered before it could be \
                     attested"
                ))),
            },
            Err(AdmitError::ArtifactRefused(reason)) => Err(LoadError::Refused(reason)),
            Err(AdmitError::Internal(reason)) => Err(LoadError::Internal(reason)),
        }
    }

    // Read the component at `path` through the read-only mount it lies
    // beneath.
    async fn read(&self, path: &str) -> Result<Vec<u8>, LoadError> {
        let (dir, subpath) = self.resolve(path)?;
        let read = tokio::task::spawn_blocking(move || {
            dir.read(&subpath).with_context(|| format!("reading component `{subpath}`"))
        });
        read.await
            .context("component read task panicked")
            .and_then(|result| result)
            .map_err(|error| LoadError::Unavailable(format!("{error:#}")))
    }

    // Resolve `path` to the mount it lies beneath and its subpath within it
    // — the longest mount-name prefix, else `.` for a bare relative path, as
    // wasi-libc resolves a guest's own opens — refusing before anything is
    // read when that mount is writable. The subpath must be plain: cap-std
    // then refuses any escape at open time.
    fn resolve(&self, path: &str) -> Result<(Arc<Dir>, String), LoadError> {
        let best = self
            .mounts
            .iter()
            .filter_map(|root| {
                if path == root.name {
                    // Naming a mount itself yields an empty subpath, which
                    // `check_subpath` refuses — kept so the refusal names the
                    // mount rather than "beneath no mount".
                    return Some((root, ""));
                }
                let subpath = path.strip_prefix(&root.name)?.strip_prefix('/')?;
                Some((root, subpath))
            })
            .max_by_key(|(root, _)| root.name.len())
            .or_else(|| self.mounts.iter().find(|root| root.name == ".").map(|root| (root, path)));
        let Some((root, subpath)) = best else {
            return Err(LoadError::Refused(format!(
                "path `{path}` is beneath no mount of this deployment"
            )));
        };
        if root.writable {
            return Err(LoadError::Refused(format!(
                "path `{path}` is beneath the writable mount `{}`; a component loads from a \
                 read-only mount alone",
                root.name
            )));
        }
        check_subpath(path, subpath)?;
        Ok((Arc::clone(&root.dir), subpath.to_owned()))
    }
}

// The deployment's mounts as load roots, refusing a writable mount that
// shares or nests a read-only mount's directory: written through the one
// view, its files would load through the other. Directories are told apart
// by identity, not path, so two mount points of one directory — a bind
// mount, a firmlink — are the one directory they are on disk.
fn roots(mounts: &MountRegistry) -> anyhow::Result<Vec<Root>> {
    // identify each mount's directory and the directories above it
    let lineages = mounts
        .entries()
        .iter()
        .map(|mount| {
            let lineage = ancestry(&mount.host_path).with_context(|| {
                format!("resolving mount `{}` at {}", mount.name, mount.host_path.display())
            })?;
            Ok((mount, lineage))
        })
        .collect::<anyhow::Result<Vec<_>>>()?;

    // refuse a writable mount at, above, or beneath a read-only one
    for (mount, lineage) in &lineages {
        for (other, other_lineage) in &lineages {
            let nested =
                lineage.contains(&other.identity) || other_lineage.contains(&mount.identity);
            if mount.writable && !other.writable && nested {
                bail!(
                    "writable mount `{}` ({}) shares its directory with the read-only mount `{}` \
                     ({}): a component beneath it could be written through one and loaded \
                     through the other",
                    mount.name,
                    mount.host_path.display(),
                    other.name,
                    other.host_path.display()
                );
            }
        }
    }

    Ok(mounts
        .entries()
        .iter()
        .map(|mount| Root {
            name: mount.name.clone(),
            dir: Arc::clone(&mount.dir),
            writable: mount.writable,
        })
        .collect())
}

// The `(device, inode)` identity of the directory at `path` and of each
// directory above it, nearest first — the identity the mount registry
// records for a mount, so a mount is found in another's lineage by it.
fn ancestry(path: &Path) -> anyhow::Result<Vec<(u64, u64)>> {
    let path = path.canonicalize()?;
    path.ancestors()
        .map(|dir| {
            let meta =
                std::fs::metadata(dir).with_context(|| format!("identifying {}", dir.display()))?;
            Ok((meta.dev(), meta.ino()))
        })
        .collect()
}

fn check_subpath(path: &str, subpath: &str) -> Result<(), LoadError> {
    // A leading '/' surfaces as an empty first segment, so the split check
    // also refuses absolute paths.
    let plain = !subpath.contains('\\')
        && subpath.split('/').all(|part| !part.is_empty() && part != "." && part != "..");
    if !plain {
        return Err(LoadError::Refused(format!(
            "component path `{path}` is not a plain relative path within a mount"
        )));
    }
    Ok(())
}

/// Loaded plugin: routed identity plus content digest.
#[derive(Clone, Debug)]
pub struct Plugin {
    id: GuestId,
    digest: Digest,
}

impl Plugin {
    /// Routed identity for host-mediated dispatch.
    #[must_use]
    pub const fn id(&self) -> &GuestId {
        &self.id
    }

    /// The content digest of the bytes the guest was loaded from.
    #[must_use]
    pub const fn digest(&self) -> Digest {
        self.digest
    }
}

#[cfg(test)]
mod tests {
    use omnia_core::ResolvedPreopen;

    use super::*;

    fn preopen(name: &str, path: &std::path::Path, writable: bool) -> ResolvedPreopen {
        ResolvedPreopen::new(name.to_owned(), path.to_path_buf(), writable)
    }

    // A writable mount nested inside a read-only one, or enclosing one, or
    // sharing its directory, is refused at install; two read-only mounts, or
    // two writable ones, may nest freely.
    #[test]
    fn overlap() {
        let dir = tempfile::tempdir().expect("scratch dir");
        let root = dir.path();
        let out = root.join("out");
        std::fs::create_dir(&out).expect("out dir");

        let open = |preopens| MountRegistry::open(preopens).expect("mounts open");
        for (first, second) in [
            (preopen(".", root, false), preopen("out", &out, true)),
            (preopen(".", root, true), preopen("out", &out, false)),
            (preopen("a", root, false), preopen("b", root, true)),
        ] {
            let error = roots(&open(vec![first, second])).expect_err("overlap refused");
            assert!(error.to_string().contains("writable mount"), "{error}");
        }
        for (first, second) in [
            (preopen(".", root, false), preopen("out", &out, false)),
            (preopen(".", root, true), preopen("out", &out, true)),
        ] {
            roots(&open(vec![first, second])).expect("same-permission nesting installs");
        }
        roots(&open(vec![preopen(".", root, false)])).expect("one read-only mount installs");
    }

    // Two mount points of one directory are refused as one directory, though
    // their canonical paths differ. macOS firmlinks the data volume at
    // `/System/Volumes/Data`, so every directory beneath it has a second
    // path with a canonical form of its own — the one such view a test can
    // reach unprivileged; a bind mount is the same case elsewhere.
    #[cfg(target_os = "macos")]
    #[test]
    fn overlap_across_firmlink() {
        let dir = tempfile::tempdir().expect("scratch dir");
        let root = dir.path().canonicalize().expect("canonical scratch dir");
        let relative = root.strip_prefix("/").expect("a canonical path is absolute");
        let alias = Path::new("/System/Volumes/Data").join(relative);
        assert_ne!(
            alias.canonicalize().expect("the firmlinked path resolves"),
            root,
            "the firmlink keeps a canonical path of its own"
        );

        let mounts =
            MountRegistry::open(vec![preopen(".", &root, false), preopen("out", &alias, true)])
                .expect("mounts open");
        let error = roots(&mounts).expect_err("one directory through two paths is refused");
        assert!(error.to_string().contains("writable mount"), "{error}");
    }

    #[test]
    fn subpaths() {
        for plain in ["tool.wasm", "adapters/tool.wasm", ".hidden/tool.wasm"] {
            check_subpath(plain, plain).expect("plain relative paths pass");
        }
        for escaping in ["", "/etc/passwd", "../tool.wasm", "./tool.wasm", "a//b", "a\\b"] {
            check_subpath(escaping, escaping).expect_err("escapes are refused");
        }
    }
}
