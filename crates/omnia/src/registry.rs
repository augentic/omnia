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

use std::collections::{BTreeMap, BTreeSet, btree_map};
use std::fmt;
use std::sync::{Arc, PoisonError, RwLock};

use anyhow::{Context as _, Result, bail, ensure};
pub use routing::{CliRoutes, HttpRoutes, PatternRoutes, Routes, TriggerRouter};
use wasmtime::Engine;
use wasmtime::component::{Component, InstancePre, Linker};
use wasmtime_wasi::WasiView;
use wrpc_wasmtime::WrpcView;

use crate::RuntimeOptions;
use crate::deployment::LoadedGuest;
use crate::dispatch::{self, DispatchHandle};

/// Opaque guest identity.
///
/// The runtime core treats it as an ordered string key; consumers (e.g. Specify)
/// project their own scheme onto it (`source:typescript`, ...). Omnia never
/// parses it.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct GuestId(Arc<str>);

impl GuestId {
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

/// A registry entry's resolution target.
///
/// Only [`Target::Local`] exists today; a remote wRPC-endpoint variant will land
/// with distributed transport.
enum Target<T: 'static> {
    /// A locally pre-instantiated component.
    Local(InstancePre<T>),
}

/// A registered guest: an opaque identity bound to a resolution target.
pub struct Guest<T: 'static> {
    id: GuestId,
    target: Target<T>,
}

impl<T: 'static> Guest<T> {
    /// Create a guest backed by a local pre-instantiated component.
    #[must_use]
    pub const fn local(id: GuestId, instance_pre: InstancePre<T>) -> Self {
        Self {
            id,
            target: Target::Local(instance_pre),
        }
    }

    /// Returns the guest's identity.
    #[must_use]
    pub const fn id(&self) -> &GuestId {
        &self.id
    }

    /// Returns the guest's pre-instantiated component, ready to instantiate
    /// fresh on a new [`wasmtime::Store`] per call.
    #[must_use]
    pub const fn instance_pre(&self) -> &InstancePre<T> {
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

/// One [`Engine`] + one `Linker`; many pre-instantiated guests keyed by
/// identity.
///
/// Every guest is pre-instantiated against the *same* linker, so they share one
/// set of host interfaces and one pooling pool — load-bearing for the
/// instance-per-call cost story. Pre-instantiation happens once, at
/// registration; per call only a fresh instantiate on a new store remains.
///
/// The guest map grows (and shrinks) after assembly through the dynamic
/// registration seam ([`Runtime::register`](crate::Runtime::register)); the
/// linker is retained so late guests pre-instantiate against the same host set.
///
/// The registry is cheap to share behind an `Arc`, matching how the runtime
/// context is cloned into each connection handler.
pub struct Registry<T: 'static> {
    engine: Engine,
    options: RuntimeOptions,
    linker: Linker<T>,
    // Concurrent-read, exclusive-write; guards are never held across an await.
    guests: RwLock<BTreeMap<GuestId, Arc<Guest<T>>>>,
    // Assemble-time identities, which deregistration refuses to remove.
    static_ids: BTreeSet<GuestId>,
    // Link interfaces polyfilled onto the shared linker at bootstrap; a late
    // guest's remaining allow-listed imports are polyfilled on a linker clone.
    wired_links: BTreeSet<Box<str>>,
    routes: Routes,
    dispatch: Arc<DispatchHandle>,
}

impl<T: WasiView + 'static> Registry<T> {
    /// Assemble a registry from a linked deployment's parts: polyfill
    /// host-mediated imports, pre-instantiate every loaded guest, validate that
    /// routes name registered guests, and freeze the static set.
    ///
    /// [`DeploymentBuilder::build`](crate::DeploymentBuilder::build) is the usual entry point.
    ///
    /// # Errors
    ///
    /// Returns an error if there are no guests to register (unless `dynamic`),
    /// host-mediated imports cannot be polyfilled, a component cannot be
    /// pre-instantiated, or a route targets a guest that is not registered.
    pub fn assemble(
        engine: Engine, mut linker: Linker<T>, options: RuntimeOptions, loaded: Vec<LoadedGuest>,
        routes: Routes, dispatch: Arc<DispatchHandle>, dynamic: bool,
    ) -> Result<Self>
    where
        T: WrpcView,
    {
        if loaded.is_empty() && !dynamic {
            bail!("cannot build a guest registry with no guests");
        }

        let wired_links = dispatch::link(&engine, &mut linker, &loaded, &dispatch)?;

        let mut guests = BTreeMap::new();
        for guest in loaded {
            let instance_pre = linker
                .instantiate_pre(&guest.component)
                .map_err(anyhow::Error::from)
                .with_context(|| format!("pre-instantiating guest `{}`", guest.id))?;
            let id = guest.id.clone();
            if guests
                .insert(guest.id.clone(), Arc::new(Guest::local(guest.id, instance_pre)))
                .is_some()
            {
                bail!("duplicate guest id `{id}`: guest identities must be unique");
            }
        }

        for target in routes.targets() {
            if !guests.contains_key(target) {
                bail!("route targets guest `{target}`, which is not registered");
            }
        }

        tracing::info!(guests = guests.len(), "runtime initialized");

        let static_ids = guests.keys().cloned().collect();
        Ok(Self {
            engine,
            options,
            linker,
            guests: RwLock::new(guests),
            static_ids,
            wired_links,
            routes,
            dispatch,
        })
    }

    /// Pre-instantiate a late (dynamically registered) component against the
    /// shared host set.
    ///
    /// Allow-listed link imports the bootstrap did not polyfill (no static
    /// guest imports them) are polyfilled on a clone of the retained linker,
    /// from this component's own import types — the shared linker is never
    /// mutated after bootstrap. Imports outside the linked host set and the
    /// `link` union fail here, exactly as at bootstrap.
    pub(crate) fn instantiate_late(
        &self, id: &GuestId, component: &Component,
    ) -> Result<InstancePre<T>>
    where
        T: WrpcView,
    {
        let mut linker = self.linker.clone();
        dispatch::polyfill_late(
            &self.engine,
            &mut linker,
            id,
            component,
            &self.dispatch,
            &self.wired_links,
        )?;
        linker
            .instantiate_pre(component)
            .map_err(anyhow::Error::from)
            .with_context(|| format!("pre-instantiating guest `{id}`"))
    }
}

impl<T: 'static> Registry<T> {
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

    /// Look up a guest by identity.
    #[must_use]
    pub fn get(&self, id: &GuestId) -> Option<Arc<Guest<T>>> {
        self.guests.read().unwrap_or_else(PoisonError::into_inner).get(id).cloned()
    }

    /// Snapshot every registered guest in a deterministic, identity-sorted
    /// order so per-trigger capability and ambiguity errors are stable across
    /// runs.
    ///
    /// The order falls out of the [`BTreeMap`] keying; no per-call sort.
    #[must_use]
    pub fn guests(&self) -> Vec<Arc<Guest<T>>> {
        self.guests.read().unwrap_or_else(PoisonError::into_inner).values().cloned().collect()
    }

    /// Publish a late guest. Refuses an identity that is already registered
    /// (static entries can never be shadowed; a dynamic upgrade is
    /// deregister + register).
    pub(crate) fn insert(&self, guest: Guest<T>) -> Result<()> {
        let id = guest.id().clone();
        let inserted = {
            let mut guests = self.guests.write().unwrap_or_else(PoisonError::into_inner);
            match guests.entry(id.clone()) {
                btree_map::Entry::Occupied(_) => false,
                btree_map::Entry::Vacant(slot) => {
                    slot.insert(Arc::new(guest));
                    true
                }
            }
        };
        ensure!(inserted, "guest `{id}` is already registered");
        Ok(())
    }

    /// Remove a dynamically registered guest. Refuses static (assemble-time)
    /// entries and unregistered identities.
    pub(crate) fn remove(&self, id: &GuestId) -> Result<()> {
        if self.static_ids.contains(id) {
            bail!("guest `{id}` is a static deployment entry and cannot be deregistered");
        }
        let removed = self.guests.write().unwrap_or_else(PoisonError::into_inner).remove(id);
        ensure!(removed.is_some(), "guest `{id}` is not registered");
        Ok(())
    }

    /// Returns the per-trigger inbound route tables built from the manifest's
    /// `[[route.*]]` sections.
    #[must_use]
    pub const fn routes(&self) -> &Routes {
        &self.routes
    }

    /// Returns the shared host-mediated dynamic-linking dispatch handle (the
    /// selector strategy, link allow-list union, and bound transport).
    #[must_use]
    pub(crate) const fn dispatch(&self) -> &Arc<DispatchHandle> {
        &self.dispatch
    }

    /// Returns the number of registered guests.
    #[must_use]
    pub fn len(&self) -> usize {
        self.guests.read().unwrap_or_else(PoisonError::into_inner).len()
    }

    /// Returns `true` if the registry has no guests.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.guests.read().unwrap_or_else(PoisonError::into_inner).is_empty()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use wasmtime::component::Linker;
    use wasmtime::{Config, Engine};

    use super::*;
    use crate::dispatch::FirstArgSelector;
    use crate::store::StoreCtx;

    fn assemble_empty(dynamic: bool) -> Result<Registry<StoreCtx<()>>, anyhow::Error> {
        let options = RuntimeOptions::load().expect("options should load");
        let engine = Engine::new(&Config::from(&options)).expect("engine should build");
        let linker = Linker::<StoreCtx<()>>::new(&engine);
        let dispatch = DispatchHandle::new(
            Arc::new(FirstArgSelector),
            BTreeSet::new(),
            8,
            std::time::Duration::from_secs(30),
        );

        Registry::assemble(
            engine,
            linker,
            options,
            Vec::new(),
            Routes::default(),
            dispatch,
            dynamic,
        )
    }

    #[test]
    fn no_guests() {
        assert!(assemble_empty(false).is_err(), "an empty static registry must be rejected");
    }

    #[test]
    fn no_guests_dynamic() {
        let registry = assemble_empty(true).expect("a dynamic deployment may start with no guests");
        assert!(registry.is_empty());
    }
}
