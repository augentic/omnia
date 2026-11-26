//! The `omnia:plugins/loader` load path.

use std::future::Future;
use std::io;
use std::sync::Arc;

use anyhow::{Context as _, bail, ensure};
use cap_std::fs::Dir;
use omnia_core::{Backends, Digest, Grants, Guest, GuestError, GuestId, MountRegistry, Runtime};
use wasm_pkg_client::Registry;

use crate::admission::Admission;
use crate::error::LoadError;
use crate::source::Location;

/// Host-side `omnia:plugins/loader.load` — embedder sugar over the runtime's
/// installed [`Plugins`] extension.
pub trait PluginLoader {
    /// Ensure the guest `from` names is active and return its handle, held
    /// to `pin` when one is given. Idempotent on (name, digest).
    ///
    /// The embedder's load is held to no caller's grants. A guest it admits
    /// by path or package runs under the deployment's `[plugins.grants]`.
    ///
    /// A load whose bytes miss `pin`, or whose name is active under other
    /// bytes, loads nothing and answers a [`LoadConflict`].
    ///
    /// # Errors
    ///
    /// - [`LoadError::Unsupported`] on a runtime with no [`Plugins`]
    ///   installed.
    /// - Any other [`LoadError`] as [`Plugins::load`] reports it.
    fn load(
        &self, from: Location, pin: Option<Digest>,
    ) -> impl Future<Output = Result<Result<Plugin, LoadConflict>, LoadError>> + Send;
}

impl<B: Backends> PluginLoader for Runtime<B> {
    fn load(
        &self, from: Location, pin: Option<Digest>,
    ) -> impl Future<Output = Result<Result<Plugin, LoadConflict>, LoadError>> + Send {
        let plugins = self.extensions().get::<Plugins>();
        async move {
            match plugins {
                Some(plugins) => plugins.load(from, pin, &Grants::all()).await,
                None => Err(LoadError::no_plugins(&from)),
            }
        }
    }
}

/// The deployment's grant over the runtime's seams: the guests it declares,
/// reached through the runtime's first-use seam; its read-only mounts; and
/// the registry source its packages are fetched through.
///
/// The grant is fixed at install. `load` refuses a location that derives a
/// name outside the loading guest's [`guests`](Grants::guests), and resolves
/// any other inside the grant:
///
/// - a declared name to the deployment's own entry;
/// - a path to the read-only mount beneath which it lies, of those the
///   loading guest reaches;
/// - a package to the registry the deployment routes it to, or to the one
///   the load names when the loading guest's grants reach its host and every
///   host its fetch leads to.
///
/// It admits the bytes it finds there through the runtime's one admission
/// body. A guest loaded by path or package runs under the deployment's
/// `[plugins.grants]`, else [`Grants::all`], within the loading guest's.
/// Nothing a caller passes widens the grant.
pub struct Plugins {
    mounts: Vec<Root>,
    grants: Grants,
    admission: Box<dyn Admission>,
}

impl Plugins {
    /// Install the loader capability on `runtime`, holding each path or
    /// package load to `grants`.
    ///
    /// A guest loaded by path or package runs under `grants` within the
    /// loading guest's. The deployment passes its `[plugins.grants]`, else
    /// [`Grants::all`].
    ///
    /// The runtime's mounts are the roots a path load resolves against. The
    /// read-only ones are code roots and the writable ones refusals. Mount
    /// directories are told apart by identity, not path, so two mount points
    /// of one directory are one directory here. The runtime's registry
    /// source fetches the packages a load names. A declared name loads
    /// through the runtime's own first-use seam, so nothing of the
    /// deployment's guests is copied here.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    ///
    /// - `grants` fail [`Runtime::validate_grants`];
    /// - a writable mount and a read-only mount share or nest their
    ///   directories;
    /// - the capability is already installed.
    #[doc(hidden)]
    pub fn install<B: Backends>(runtime: &Runtime<B>, grants: Grants) -> anyhow::Result<()> {
        runtime.validate_grants(&grants).context("the guest loader's grants are invalid")?;
        let plugins = Self {
            mounts: roots(runtime.mounts())?,
            grants,
            admission: Box::new(runtime.downgrade()),
        };
        ensure!(
            runtime.extensions().insert(plugins),
            "the plugins capability installs exactly once per runtime"
        );
        Ok(())
    }

    /// Ensure the guest `from` names is active and return its handle, loading
    /// it for a guest that runs under `caller`.
    ///
    /// The name the location derives must be one of `caller`'s
    /// [`guests`](Grants::guests), whatever the location.
    ///
    /// - A declared name goes through the runtime's first-use seam. The
    ///   deployment's entry is its source, its pin, and its grants. A guest
    ///   already loaded is attested.
    /// - A path or package is acquired inside the deployment's grant: a path
    ///   beneath a mount `caller` reaches, and a package from a registry the
    ///   load names only when `caller`'s [`hosts`](Grants::hosts) reach that
    ///   registry's host and every host its fetch leads to. It is admitted
    ///   through the runtime's one admission body as caller-named bytes: held
    ///   to `pin` when one is given, raw wasm alone, under the name the
    ///   location derives, which no entry of the deployment may declare. The
    ///   guest runs under the deployment's `[plugins.grants]`, within
    ///   `caller`. A name already active under the same bytes and grants is
    ///   attested.
    ///
    /// Idempotent on (name, digest). A path or package whose bytes miss
    /// `pin`, or whose name is active under other bytes, loads nothing and
    /// answers a [`LoadConflict`].
    ///
    /// # Errors
    ///
    /// - [`LoadError::NotFound`] for an undeclared name, a path beneath no
    ///   mount or with no file, or a package nothing routes or holds.
    /// - [`LoadError::Denied`] for:
    ///   - a location deriving a name outside `caller`'s
    ///     [`guests`](Grants::guests);
    ///   - a path or package deriving a name the deployment declares, or one
    ///     active under other grants;
    ///   - a path beneath a mount `caller` does not reach, beneath a writable
    ///     mount, or one the filesystem refuses to read;
    ///   - a registry named for a package the deployment routes elsewhere;
    ///   - a registry named whose host, or a host its fetch leads to, `caller`
    ///     does not reach;
    ///   - a component importing code from a package its grants do not
    ///     reach.
    /// - [`LoadError::Invalid`] for a declared name with a pin, a malformed
    ///   path, package, or registry, or bytes that miss the deployment's
    ///   pin, are pre-compiled where raw wasm alone is admitted, or are not
    ///   a loadable component.
    /// - [`LoadError::Unsupported`] for a package the runtime cannot fetch.
    /// - [`LoadError::Unavailable`] when the mount, the store, or the
    ///   registry did not produce the bytes.
    /// - [`LoadError::Other`] when the registration failed, the runtime has
    ///   shut down, or a declared guest's load panicked.
    pub async fn load(
        &self, from: Location, pin: Option<Digest>, caller: &Grants,
    ) -> Result<Result<Plugin, LoadConflict>, LoadError> {
        // hold the name the location derives to the caller's guests
        let id = from.id();
        if !caller.allows_guest(id.as_str()) {
            return Err(LoadError::Denied(format!(
                "the loading guest's grants reach no guest `{id}`, which `{from}` names"
            )));
        }

        // acquire the bytes the location names, inside the grant; a name the
        // runtime refuses caller-named bytes under is refused before any read
        let bytes = match &from {
            Location::Declared(name) => {
                if pin.is_some() {
                    return Err(LoadError::Invalid(format!(
                        "`{name}` is declared by the deployment, which pins it; the load takes \
                         no digest"
                    )));
                }
                let plugin = self.admission.guest(&id).await?;
                tracing::info!(%id, digest = %plugin.digest, "declared guest loaded");
                return Ok(Ok(plugin));
            }
            Location::Path(path) => {
                self.admission.admits(&id)?;
                self.read(path, caller).await?
            }
            Location::Registry { package, endpoint } => {
                self.admission.admits(&id)?;
                if let Some(endpoint) = endpoint {
                    check_registry(endpoint, caller)?;
                }
                self.admission.acquire(package, endpoint.as_deref(), caller).await?
            }
        };

        // admit under the deployment's grants within the caller's, which a refusal names
        let grants = self.grants.within(caller);
        let admitted = self.admission.admit_bytes(id.clone(), bytes, pin, grants).await;
        let loaded = admitted.map_err(|error| match error {
            GuestError::Denied(reason) => LoadError::Denied(format!(
                "{reason}; a guest loaded by path or package runs under the deployment's \
                 `[plugins.grants]` within the loading guest's grants"
            )),
            error => LoadError::from(error),
        })?;
        match &loaded {
            Ok(plugin) => tracing::info!(%id, digest = %plugin.digest, "guest loaded"),
            Err(conflict) => tracing::debug!(%id, found = %conflict.found, "guest load conflicted"),
        }
        Ok(loaded)
    }

    // Read the component at `path` through the read-only mount it lies
    // beneath, of those `caller` reaches.
    async fn read(&self, path: &str, caller: &Grants) -> Result<Vec<u8>, LoadError> {
        let (dir, subpath) = self.resolve(path, caller)?;
        let read = tokio::task::spawn_blocking(move || {
            dir.read(&subpath).map_err(|error| {
                let detail = format!("reading component `{subpath}`: {error}");
                match error.kind() {
                    io::ErrorKind::NotFound => LoadError::NotFound(detail),
                    io::ErrorKind::PermissionDenied => LoadError::Denied(detail),
                    _ => LoadError::Unavailable(detail),
                }
            })
        });
        read.await.unwrap_or_else(|error| {
            Err(LoadError::Other(format!("the component read task failed: {error}")))
        })
    }

    // The mount `path` lies beneath, as the loading guest resolves its own
    // opens over the mounts its grants reach, refused before any read when
    // writable; cap-std refuses any escape in the subpath at open time.
    fn resolve(&self, path: &str, caller: &Grants) -> Result<(Arc<Dir>, String), LoadError> {
        let reached: Vec<&Root> =
            self.mounts.iter().filter(|root| caller.allows_mount(&root.name)).collect();
        let Some((root, subpath)) = beneath(&reached, path) else {
            let all: Vec<&Root> = self.mounts.iter().collect();
            return Err(match beneath(&all, path) {
                Some((root, _)) => LoadError::Denied(format!(
                    "path `{path}` is beneath the mount `{}`, which the loading guest's grants \
                     do not reach",
                    root.name
                )),
                None => LoadError::NotFound(format!(
                    "path `{path}` is beneath no mount of this deployment"
                )),
            });
        };
        if root.writable {
            return Err(LoadError::Denied(format!(
                "path `{path}` is beneath the writable mount `{}`; a component loads from a \
                 read-only mount alone",
                root.name
            )));
        }
        check_subpath(path, subpath)?;
        Ok((Arc::clone(&root.dir), subpath.to_owned()))
    }
}

// The registry a load names is parsed before the caller's `hosts` are read,
// so a malformed one is invalid whatever that list holds.
fn check_registry(endpoint: &str, caller: &Grants) -> Result<(), LoadError> {
    let registry = endpoint.parse::<Registry>().map_err(|error| {
        LoadError::Invalid(format!("registry `{endpoint}` is not a valid name: {error}"))
    })?;
    if caller.allows_host(registry.host()) {
        Ok(())
    } else {
        Err(LoadError::Denied(format!(
            "registry `{endpoint}` is a host the loading guest's grants do not reach"
        )))
    }
}

// The root `path` lies beneath as wasi-libc resolves a guest's opens: the
// longest name prefix, else `.`.
fn beneath<'a>(roots: &[&'a Root], path: &'a str) -> Option<(&'a Root, &'a str)> {
    roots
        .iter()
        .filter_map(|root| {
            if path == root.name {
                // kept so `check_subpath`'s refusal names the mount
                return Some((*root, ""));
            }
            let subpath = path.strip_prefix(&root.name)?.strip_prefix('/')?;
            Some((*root, subpath))
        })
        .max_by_key(|(root, _)| root.name.len())
        .or_else(|| roots.iter().find(|root| root.name == ".").map(|root| (*root, path)))
}

// One of the deployment's mounts as a path load sees it: the handle it
// reads through, and whether the guest can write beneath it.
struct Root {
    name: String,
    dir: Arc<Dir>,
    writable: bool,
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
        return Err(LoadError::Invalid(format!(
            "component path `{path}` is not a plain relative path within a mount"
        )));
    }
    Ok(())
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

/// A load that loaded nothing because its precondition did not hold.
///
/// The bytes missed the load's pin, or a guest is active under the name from
/// other bytes.
#[derive(Clone, Debug)]
pub struct LoadConflict {
    found: Digest,
    current: Option<Plugin>,
}

impl LoadConflict {
    pub(crate) const fn new(found: Digest, current: Option<Plugin>) -> Self {
        Self { found, current }
    }

    /// The digest of the bytes the location names.
    #[must_use]
    pub const fn found(&self) -> Digest {
        self.found
    }

    /// The guest active under the name, `None` when none is.
    #[must_use]
    pub const fn current(&self) -> Option<&Plugin> {
        self.current.as_ref()
    }
}
