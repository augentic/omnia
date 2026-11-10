//! The `omnia:plugins/loader` load path.

use std::future::Future;
use std::sync::Arc;

use anyhow::{Context as _, bail, ensure};
use cap_std::fs::Dir;
use omnia_core::{Backends, Digest, Guest, GuestId, MountRegistry, Runtime};

use crate::admission::Admission;
use crate::error::LoadError;
use crate::source::Location;

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

impl<B: Backends> PluginLoader for Runtime<B> {
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

/// The deployment's grant over the runtime's seams: the guests it declares,
/// reached through the runtime's first-use seam; its read-only mounts; and
/// the registry source its packages are fetched through.
///
/// The grant is fixed at install. `load` resolves a location inside it — a
/// declared name to the deployment's own entry, a path to the read-only
/// mount beneath which it lies, a package to the registry the deployment
/// routes it to — and admits the bytes it finds there through the runtime's
/// one admission body. Nothing a caller passes widens it.
pub struct Plugins {
    mounts: Vec<Root>,
    admission: Box<dyn Admission>,
}

impl Plugins {
    /// Install the loader capability on `runtime`: its mounts are the roots
    /// a path load resolves against — the read-only ones as code roots, the
    /// writable ones as refusals — and its registry source fetches the
    /// packages a load names. Mount directories are told apart by identity,
    /// not path: two mount points of one directory are one directory here. A
    /// declared name loads through the runtime's own first-use seam, so
    /// nothing of the deployment's guests is copied here.
    ///
    /// # Errors
    ///
    /// Returns an error if a writable mount and a read-only mount share or
    /// nest their directories, or the capability is already installed.
    pub fn install<B: Backends>(runtime: &Runtime<B>) -> anyhow::Result<()> {
        let plugins = Self {
            mounts: roots(runtime.mounts())?,
            admission: Box::new(runtime.downgrade()),
        };
        ensure!(
            runtime.extensions().insert(plugins),
            "the plugins capability installs exactly once per runtime"
        );
        Ok(())
    }

    /// Ensure the guest `from` names is active and return its handle. A
    /// declared name goes through the runtime's first-use seam: the
    /// deployment's entry is its source and its pin, and a guest already
    /// loaded is attested. A path or package is acquired inside the
    /// deployment's grant and admitted through the runtime's one admission
    /// body as caller-named bytes — held to `pin` when one is given, raw
    /// wasm alone, under the name the location derives, which no entry of
    /// the deployment may declare; a name already active under the same
    /// bytes is attested. Idempotent on (name, digest).
    ///
    /// # Errors
    ///
    /// `refused` on an undeclared name, a declared name with a pin, a path
    /// or package deriving a name the deployment declares, a path beneath no
    /// read-only mount, a package nothing routes, a digest mismatch, a
    /// pre-compiled artifact where raw wasm alone is admitted (a
    /// caller-named path or package, a declared package, or a declared path
    /// without a `digest`), bytes that are not a loadable component, or a
    /// name active under other bytes; `unavailable` when the source could
    /// not produce the bytes; `internal` on registration failure.
    pub async fn load(&self, from: Location, pin: Option<Digest>) -> Result<Plugin, LoadError> {
        let id = from.id();

        // acquire the bytes the location names, inside the grant; a name the
        // runtime refuses caller-named bytes under is refused before any read
        let bytes = match &from {
            Location::Declared(name) => {
                if pin.is_some() {
                    return Err(LoadError::Refused(format!(
                        "`{name}` is declared by the deployment, which pins it; the load takes \
                         no digest"
                    )));
                }
                let plugin = self.admission.guest(&id).await?;
                tracing::info!(%id, digest = %plugin.digest, "declared guest loaded");
                return Ok(plugin);
            }
            Location::Path(path) => {
                self.admission.admits(&id)?;
                self.read(path).await?
            }
            Location::Registry { package, endpoint } => {
                self.admission.admits(&id)?;
                self.admission.acquire(package, endpoint.as_deref()).await?
            }
        };

        let plugin = self.admission.admit_bytes(id.clone(), bytes, pin).await?;
        tracing::info!(%id, digest = %plugin.digest, "guest loaded");
        Ok(plugin)
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

    // The mount `path` lies beneath, as wasi-libc resolves a guest's own opens
    // (longest name prefix, else `.`), refused before any read when writable;
    // cap-std refuses any escape in the subpath at open time.
    fn resolve(&self, path: &str) -> Result<(Arc<Dir>, String), LoadError> {
        let best = self
            .mounts
            .iter()
            .filter_map(|root| {
                if path == root.name {
                    // kept so `check_subpath`'s refusal names the mount
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

// One of the deployment's mounts as a path load sees it: the handle it
// reads through, and whether the guest can write beneath it.
#[derive(Debug)]
struct Root {
    name: String,
    dir: Arc<Dir>,
    writable: bool,
}

/// Loaded plugin: routed identity, content digest, and what it exports.
#[derive(Clone, Debug)]
pub struct Plugin {
    id: GuestId,
    digest: Digest,
    exports: Arc<[String]>,
}

impl Plugin {
    /// The handle of a seated guest.
    #[must_use]
    pub fn of<B: 'static>(guest: &Guest<B>) -> Self {
        Self {
            id: guest.id().clone(),
            digest: guest.digest(),
            exports: Arc::clone(guest.exports()),
        }
    }

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

    /// The names the component exports, as [`Guest::exports`] lists them.
    #[must_use]
    pub fn exports(&self) -> &[String] {
        &self.exports
    }
}

// Refuses a writable mount that shares or nests a read-only mount's
// directory: written through one view, its files would load through the
// other. Directories are told apart by identity, not path.
fn roots(mounts: &MountRegistry) -> anyhow::Result<Vec<Root>> {
    // identify each mount's directory and the directories above it
    let lineages = mounts
        .entries()
        .iter()
        .map(|mount| {
            let lineage = MountRegistry::ancestry(&mount.host_path).with_context(|| {
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
