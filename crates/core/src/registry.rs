//! # Guest registry
//!
//! One [`Engine`] and one `Linker` hold many pre-instantiated guests at once,
//! each selectable by an opaque [`GuestId`]. A registry entry is instantiated
//! fresh per call and discarded (instance-per-call). This is pure wasmtime
//! infrastructure: it is what lets one process route an HTTP request, a CLI
//! command, and a topic message to *different* guests.
//!
//! The runtime core treats identities as opaque keys; consumers project their own
//! scheme onto them. Omnia never parses a [`GuestId`].

mod routing;

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::Path;
use std::sync::{Arc, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};

use anyhow::{Context as _, Result, bail, ensure};
pub use routing::{HttpRoutes, PatternRoutes, Resolver, Routes, TriggerRouter};
use wasmtime::Engine;
use wasmtime::component::types::ComponentItem;
use wasmtime::component::{Component, InstancePre, Linker};

use crate::digest::Digest;
use crate::seam::LinkSeam;
use crate::source::{LoadedGuest, Source};
use crate::store::StoreCtx;
use crate::{Grants, RuntimeOptions};

/// Opaque guest identity.
///
/// The runtime core treats it as an ordered string key; consumers (e.g. Specify)
/// project their own scheme onto it (`source:typescript`, ...). Omnia never
/// parses it.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct GuestId(Arc<str>);

impl GuestId {
    /// Returns the identity a component path names: its file stem.
    ///
    /// A path with no stem (`..`, an empty string) names itself.
    ///
    /// # Examples
    ///
    /// ```
    /// use omnia_core::GuestId;
    ///
    /// assert_eq!(GuestId::from_path("./adapters/intent.wasm").as_str(), "intent");
    /// assert_eq!(GuestId::from_path("intent").as_str(), "intent");
    /// ```
    #[must_use]
    pub fn from_path(path: &str) -> Self {
        Path::new(path)
            .file_stem()
            .and_then(|stem| stem.to_str())
            .map_or_else(|| Self::from(path), Self::from)
    }

    /// Returns the identity a package reference names: the reference without
    /// its version.
    ///
    /// # Examples
    ///
    /// ```
    /// use omnia_core::GuestId;
    ///
    /// assert_eq!(GuestId::from_package("acme:tool@1.2.3").as_str(), "acme:tool");
    /// assert_eq!(GuestId::from_package("acme:tool").as_str(), "acme:tool");
    /// ```
    #[must_use]
    pub fn from_package(reference: &str) -> Self {
        Self::from(reference.split_once('@').map_or(reference, |(name, _)| name))
    }

    /// Returns the identity as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for GuestId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<&str> for GuestId {
    fn from(value: &str) -> Self {
        Self(Arc::from(value))
    }
}

impl From<String> for GuestId {
    fn from(value: String) -> Self {
        Self(Arc::from(value))
    }
}

/// A registered guest over the backend bundle `B`: an opaque identity bound
/// to a resolution target.
pub struct Guest<B: 'static> {
    id: GuestId,
    target: Target<B>,
    digest: Digest,
    exports: Arc<[String]>,
    grants: Arc<Grants>,
}

impl<B: 'static> Guest<B> {
    // the digest is of the bytes the guest was loaded from, so the
    // attestation lives and dies with the registry entry itself
    #[must_use]
    pub(crate) fn local(
        id: GuestId, instance_pre: InstancePre<StoreCtx<B>>, digest: Digest, grants: Grants,
    ) -> Self {
        let component = instance_pre.component();
        let exports = component
            .component_type()
            .exports(component.engine())
            .map(|(name, _)| name.to_owned())
            .collect();
        Self {
            id,
            target: Target::Local(instance_pre),
            digest,
            exports,
            grants: Arc::new(grants),
        }
    }

    /// Returns the guest's identity.
    #[must_use]
    pub const fn id(&self) -> &GuestId {
        &self.id
    }

    /// The content digest of the bytes the guest was loaded from.
    #[must_use]
    pub const fn digest(&self) -> Digest {
        self.digest
    }

    /// The names the component exports — its interfaces
    /// (`wasi:cli/run@0.3.2`) and bare functions — as the component type
    /// lists them, so a caller can tell what a guest serves before any
    /// dispatch.
    #[must_use]
    pub const fn exports(&self) -> &Arc<[String]> {
        &self.exports
    }

    /// What of the deployment the guest reaches.
    #[must_use]
    pub const fn grants(&self) -> &Arc<Grants> {
        &self.grants
    }

    /// Returns the guest's pre-instantiated component, ready to instantiate
    /// fresh on a new [`wasmtime::Store`] per call.
    #[must_use]
    pub const fn instance_pre(&self) -> &InstancePre<StoreCtx<B>> {
        match &self.target {
            Target::Local(pre) => pre,
        }
    }

    /// Returns the underlying component, used to introspect a guest's exported
    /// interfaces when wiring the host-mediated link serve side.
    #[must_use]
    pub fn component(&self) -> &Component {
        self.instance_pre().component()
    }
}

// a remote variant will land with distributed transport
enum Target<B: 'static> {
    Local(InstancePre<StoreCtx<B>>),
}

/// Inputs to [`Registry::assemble`], over the backend bundle `B`.
#[doc(hidden)]
pub struct RegistryParts<B: 'static> {
    /// The shared engine.
    pub engine: Engine,
    /// The linker every guest pre-instantiates against, hosts linked.
    pub linker: Linker<StoreCtx<B>>,
    /// Runtime options.
    pub options: RuntimeOptions,
    /// The guests loaded at boot.
    pub loaded: Vec<LoadedGuest>,
    /// The deployment's other guests, each loaded from its source at first
    /// use.
    pub declared: Vec<Source>,
    /// Per-trigger inbound route tables.
    pub routes: Routes,
    /// The guest→guest link seam.
    pub seam: Arc<dyn LinkSeam<B>>,
    /// Whether a deployment with no guests assembles (a dynamic deployment
    /// registers them later).
    pub allow_empty: bool,
}

/// One [`Engine`] + one `Linker` over the backend bundle `B`; many
/// pre-instantiated guests keyed by identity.
///
/// Every guest is pre-instantiated against the *same* linker, so they share one
/// set of host interfaces and one pooling pool — load-bearing for the
/// instance-per-call cost story. Pre-instantiation happens once, at
/// registration; per call only a fresh instantiate on a new store remains.
///
/// The guest map grows (and shrinks) after assembly through the dynamic
/// registration seam ([`Runtime::register`](crate::Runtime::register)) and
/// through first use of a declared guest ([`Runtime::guest`]); the linker is
/// retained so late guests pre-instantiate against the same host set.
///
/// The registry is cheap to share behind an `Arc`, matching how the runtime
/// context is cloned into each connection handler.
///
/// [`Runtime::guest`]: crate::Runtime::guest
pub struct Registry<B: 'static> {
    engine: Engine,
    options: RuntimeOptions,
    linker: Linker<StoreCtx<B>>,
    guests: RwLock<BTreeMap<GuestId, Arc<Guest<B>>>>,
    // gates lifecycle transitions so the guest map and the seam's live
    // endpoints change as one step; lock order is this gate, then one inner
    // map, never across an await
    lifecycle: RwLock<()>,
    // assemble-time identities, which deregistration refuses to remove
    static_ids: BTreeSet<GuestId>,
    // not loaded at boot: each loads from its source at first use
    declared: BTreeMap<GuestId, Source>,
    routes: Routes,
    seam: Arc<dyn LinkSeam<B>>,
}

impl<B: 'static> Registry<B> {
    /// Assemble a registry from a linked deployment's parts: polyfill link
    /// imports through the seam, pre-instantiate every loaded guest, check
    /// their linked signatures against each other, table the declared ones,
    /// validate that routes name guests, and freeze the static set.
    ///
    /// # Errors
    ///
    /// Returns an error if there are no guests at all (unless `allow_empty`),
    /// a guest was built for WASI 0.2 (`wasm32-wasip2`), a guest imports code
    /// from a package its grants do not reach, link imports cannot be
    /// polyfilled, a component cannot be pre-instantiated, an identity
    /// repeats, the seam's [`check`](LinkSeam::check) refuses a pair of
    /// loaded guests, or a route targets a guest that is neither loaded nor
    /// declared.
    #[doc(hidden)]
    pub fn assemble(parts: RegistryParts<B>) -> Result<Self> {
        let RegistryParts {
            engine,
            mut linker,
            options,
            loaded,
            declared,
            routes,
            seam,
            allow_empty,
        } = parts;
        if loaded.is_empty() && declared.is_empty() && !allow_empty {
            bail!("cannot build a guest registry with no guests");
        }

        for guest in &loaded {
            ensure_wasi_03(&guest.id, &guest.component, &engine)?;
            ensure_granted(&guest.id, &guest.component, &engine, &guest.grants)?;
        }
        seam.polyfill(&engine, &mut linker, &loaded)?;

        let mut guests = BTreeMap::new();
        for LoadedGuest {
            id,
            component,
            digest,
            grants,
        } in loaded
        {
            let instance_pre = linker
                .instantiate_pre(&component)
                .map_err(anyhow::Error::from)
                .with_context(|| format!("pre-instantiating guest `{id}`"))?;
            let guest = Guest::local(id.clone(), instance_pre, digest, grants);
            if guests.insert(id.clone(), Arc::new(guest)).is_some() {
                bail!("duplicate guest id `{id}`: guest identities must be unique");
            }
        }

        // each loaded guest against those before it, so every pair is checked once
        let mut checked: Vec<&Guest<B>> = Vec::with_capacity(guests.len());
        for guest in guests.values().map(Arc::as_ref) {
            seam.check(guest, &checked)?;
            checked.push(guest);
        }

        let mut sources = BTreeMap::new();
        for source in declared {
            let id = source.id().clone();
            if guests.contains_key(&id) || sources.insert(id.clone(), source).is_some() {
                bail!("duplicate guest id `{id}`: guest identities must be unique");
            }
        }

        for target in routes.targets() {
            if !guests.contains_key(target) && !sources.contains_key(target) {
                bail!("route targets guest `{target}`, which is not registered");
            }
        }

        tracing::debug!(guests = guests.len(), declared = sources.len(), "runtime initialized");

        let static_ids = guests.keys().cloned().collect();
        Ok(Self {
            engine,
            options,
            linker,
            guests: RwLock::new(guests),
            lifecycle: RwLock::new(()),
            static_ids,
            declared: sources,
            routes,
            seam,
        })
    }

    /// Pre-instantiate a late (dynamically registered) component against the
    /// shared host set.
    ///
    /// Link functions the bootstrap did not polyfill (no static guest imports
    /// them) are polyfilled by the seam on a clone of the retained linker,
    /// from this component's own import types — the shared linker is never
    /// mutated after bootstrap. An import that no linked host serves and the
    /// seam does not relay fails here, exactly as at bootstrap.
    pub(crate) fn instantiate_late(
        &self, id: &GuestId, component: &Component,
    ) -> Result<InstancePre<StoreCtx<B>>> {
        ensure_wasi_03(id, component, &self.engine)?;
        let mut linker = self.linker.clone();
        self.seam.polyfill_late(&self.engine, &mut linker, id, component)?;
        linker
            .instantiate_pre(component)
            .map_err(anyhow::Error::from)
            .with_context(|| format!("pre-instantiating guest `{id}`"))
    }

    /// Returns the shared engine every guest is instantiated against.
    #[must_use]
    pub const fn engine(&self) -> &Engine {
        &self.engine
    }

    /// Returns the runtime options.
    #[must_use]
    pub const fn options(&self) -> &RuntimeOptions {
        &self.options
    }

    // lookups under this guard never observe a half-applied transition
    fn lifecycle_read(&self) -> RwLockReadGuard<'_, ()> {
        self.lifecycle.read().unwrap_or_else(PoisonError::into_inner)
    }

    /// Enter a lifecycle write section: the holder may mutate the guest map
    /// and the seam's live endpoints as one atomic transition.
    pub(crate) fn lifecycle_write(&self) -> RwLockWriteGuard<'_, ()> {
        self.lifecycle.write().unwrap_or_else(PoisonError::into_inner)
    }

    /// Look up a registered guest by identity; a declared guest not yet
    /// loaded is `None` here and loads through
    /// [`Runtime::guest`](crate::Runtime::guest).
    #[must_use]
    pub fn get(&self, id: &GuestId) -> Option<Arc<Guest<B>>> {
        let _lifecycle = self.lifecycle_read();
        self.guests.read().unwrap_or_else(PoisonError::into_inner).get(id).cloned()
    }

    /// The source a declared guest loads from at first use, whether or not it
    /// is loaded now.
    #[must_use]
    pub fn declared(&self, id: &GuestId) -> Option<&Source> {
        self.declared.get(id)
    }

    /// Whether the deployment declares `id` as a guest loaded at first use.
    #[must_use]
    pub fn is_declared(&self, id: &GuestId) -> bool {
        self.declared.contains_key(id)
    }

    /// Snapshot every registered guest in a deterministic, identity-sorted
    /// order so per-trigger capability and ambiguity errors are stable across
    /// runs.
    ///
    /// The order falls out of the [`BTreeMap`] keying; no per-call sort.
    pub fn guests(&self) -> impl ExactSizeIterator<Item = Arc<Guest<B>>> {
        let snapshot: Vec<Arc<Guest<B>>> = {
            let _lifecycle = self.lifecycle_read();
            self.guests.read().unwrap_or_else(PoisonError::into_inner).values().cloned().collect()
        };
        snapshot.into_iter()
    }

    /// Publish a late guest and its pending link endpoint (parked by the serve
    /// side) as one lifecycle transition. Refuses an identity that is already
    /// registered (static entries can never be shadowed; a dynamic upgrade is
    /// deregister + register), and a guest the seam's
    /// [`check`](LinkSeam::check) refuses against every registered guest,
    /// discarding the pending endpoint either way; on refusal the registry map
    /// is untouched, so a failed registration leaves no partial state.
    pub(crate) fn publish(&self, guest: Arc<Guest<B>>) -> Result<(), PublishError> {
        let id = guest.id().clone();

        // lifecycle write first, then the inner maps
        let _lifecycle = self.lifecycle_write();
        let mut guests = self.guests.write().unwrap_or_else(PoisonError::into_inner);
        if guests.contains_key(&id) {
            self.seam.discard(&id);
            return Err(PublishError::Occupied);
        }

        // under the guards, so of two guests seated at once the later checks against the earlier
        let standing: Vec<&Guest<B>> = guests.values().map(Arc::as_ref).collect();
        if let Err(error) = self.seam.check(&guest, &standing) {
            self.seam.discard(&id);
            return Err(PublishError::Skew(error));
        }

        // seam before entry, so a refused publish leaves the map untouched
        self.seam.publish(&id).map_err(PublishError::Transport)?;
        guests.insert(id, guest);
        drop(guests);
        Ok(())
    }

    /// Remove a dynamically registered guest and its link endpoint as one
    /// lifecycle transition. Refuses static (assemble-time) entries and
    /// unregistered identities.
    pub(crate) fn remove(&self, id: &GuestId) -> Result<()> {
        if self.static_ids.contains(id) {
            bail!("guest `{id}` is a static deployment entry and cannot be deregistered");
        }

        let _lifecycle = self.lifecycle_write();
        let removed = self.guests.write().unwrap_or_else(PoisonError::into_inner).remove(id);
        ensure!(removed.is_some(), "guest `{id}` is not registered");
        self.seam.remove(id);
        Ok(())
    }

    /// Returns the per-trigger inbound route tables built from the manifest
    /// guests' `routes` lists.
    #[must_use]
    pub const fn routes(&self) -> &Routes {
        &self.routes
    }

    /// Returns the guest→guest link seam the registry drives.
    #[must_use]
    pub(crate) const fn seam(&self) -> &Arc<dyn LinkSeam<B>> {
        &self.seam
    }

    /// Returns the number of registered guests.
    #[must_use]
    pub fn len(&self) -> usize {
        let _lifecycle = self.lifecycle_read();
        self.guests.read().unwrap_or_else(PoisonError::into_inner).len()
    }

    /// Returns `true` if the registry has no guests.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        let _lifecycle = self.lifecycle_read();
        self.guests.read().unwrap_or_else(PoisonError::into_inner).is_empty()
    }
}

// names the rebuild a guest needs, in place of the linker's first unmet import
fn ensure_wasi_03(id: &GuestId, component: &Component, engine: &Engine) -> Result<()> {
    let component_type = component.component_type();
    let imports = component_type.imports(engine).map(|(name, _)| name);
    if let Some(import) = wasi_02_import(imports) {
        bail!(
            "guest `{id}` targets WASI 0.2 (wasm32-wasip2), importing `{import}`; rebuild it for \
             wasm32-wasip3"
        );
    }
    Ok(())
}

// the packages wasmtime-wasi served at 0.2
const WASI_02_PACKAGES: [&str; 7] = [
    "wasi:cli/",
    "wasi:clocks/",
    "wasi:filesystem/",
    "wasi:http/",
    "wasi:io/",
    "wasi:random/",
    "wasi:sockets/",
];

fn wasi_02_import<'a>(imports: impl IntoIterator<Item = &'a str>) -> Option<&'a str> {
    imports.into_iter().find(|import| {
        import.split_once('@').is_some_and(|(interface, version)| {
            version.starts_with("0.2.")
                && WASI_02_PACKAGES.iter().any(|package| interface.starts_with(package))
        })
    })
}

// one refusal names every ungranted package, so a first `interfaces` list
// is completed in one pass
pub fn ensure_granted(
    id: &GuestId, component: &Component, engine: &Engine, grants: &Grants,
) -> Result<()> {
    let component_type = component.component_type();
    let ungranted: BTreeSet<&str> = component_type
        .imports(engine)
        .filter(|(_, import)| carries_code(&import.ty, engine))
        .map(|(name, _)| package(name))
        .filter(|package| !grants.allows_interface(package))
        .collect();
    if !ungranted.is_empty() {
        let packages: Vec<String> =
            ungranted.iter().map(|package| format!("`{package}`")).collect();
        bail!(
            "guest `{id}` imports code from packages its grants do not reach: add {} to its \
             `interfaces`",
            packages.join(", ")
        );
    }
    Ok(())
}

// fails closed: an instance carries code unless every export is a type
fn carries_code(item: &ComponentItem, engine: &Engine) -> bool {
    match item {
        ComponentItem::ComponentInstance(instance) => {
            !instance.exports(engine).all(|(_, export)| matches!(export.ty, ComponentItem::Type(_)))
        }
        ComponentItem::Type(_) => false,
        ComponentItem::ComponentFunc(_)
        | ComponentItem::CoreFunc(_)
        | ComponentItem::Module(_)
        | ComponentItem::Component(_)
        | ComponentItem::Resource(_) => true,
    }
}

// `wasi:http` of `wasi:http/types@0.3.0`; a plain name is its own package
fn package(import: &str) -> &str {
    let name = import.split_once('@').map_or(import, |(name, _)| name);
    name.split_once('/').map_or(name, |(package, _)| package)
}

/// Why [`Registry::publish`] refused a late guest; `Occupied` is distinct so
/// callers can treat an identity conflict as a race, not a fault.
#[derive(Debug)]
pub enum PublishError {
    /// The identity is already registered.
    Occupied,
    /// A linked function's signature differs from a registered guest's.
    Skew(anyhow::Error),
    /// Link-endpoint installation failed.
    Transport(anyhow::Error),
}

// a private rule over import names, since no guest pipeline builds for
// `wasm32-wasip2` any more
#[cfg(test)]
mod tests {
    use super::wasi_02_import;

    #[test]
    fn wasip2_guest() {
        let imports =
            ["omnia:otel/traces@0.2.0", "wasi:cli/environment@0.2.6", "wasi:io/streams@0.2.6"];
        assert_eq!(wasi_02_import(imports), Some("wasi:cli/environment@0.2.6"));
    }

    #[test]
    fn wasip3_guest() {
        let imports = [
            "wasi:cli/environment@0.3.2",
            "wasi:clocks/system-clock@0.3.2",
            "omnia:config/store@0.1.0",
            "omnia:keyvalue/store@0.1.0",
            "omnia:messaging/producer@0.1.0",
            "omnia:sql/database@0.1.0",
            "omnia:otel/traces@0.2.0",
        ];
        assert_eq!(wasi_02_import(imports), None);
    }
}
