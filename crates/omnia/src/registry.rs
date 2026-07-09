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

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use anyhow::{Context as _, Result, bail};
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
/// The registry is cheap to share behind an `Arc`, matching how the runtime
/// context is cloned into each connection handler.
pub struct Registry<T: 'static> {
    engine: Engine,
    options: RuntimeOptions,
    guests: BTreeMap<GuestId, Guest<T>>,
    routes: Routes,
    dispatch: Arc<DispatchHandle>,
}

impl<T: WasiView + 'static> Registry<T> {
    /// Assemble a registry from a linked deployment's parts: polyfill
    /// host-mediated imports, pre-instantiate every loaded guest, validate that
    /// routes name registered guests, and freeze the result.
    ///
    /// [`DeploymentBuilder::build`](crate::DeploymentBuilder::build) is the usual entry point.
    ///
    /// # Errors
    ///
    /// Returns an error if there are no guests to register, host-mediated
    /// imports cannot be polyfilled, a component cannot be pre-instantiated, or
    /// a route targets a guest that is not registered.
    pub fn assemble(
        engine: Engine, mut linker: Linker<T>, options: RuntimeOptions, loaded: Vec<LoadedGuest>,
        routes: Routes, dispatch: Arc<DispatchHandle>,
    ) -> Result<Self>
    where
        T: WrpcView,
    {
        if loaded.is_empty() {
            bail!("cannot build a guest registry with no guests");
        }

        dispatch::link(&engine, &mut linker, &loaded, &dispatch)?;

        let mut guests = BTreeMap::new();
        for guest in loaded {
            let instance_pre = linker
                .instantiate_pre(&guest.component)
                .map_err(anyhow::Error::from)
                .with_context(|| format!("pre-instantiating guest `{}`", guest.id))?;
            let id = guest.id.clone();
            if guests.insert(guest.id.clone(), Guest::local(guest.id, instance_pre)).is_some() {
                bail!("duplicate guest id `{id}`: guest identities must be unique");
            }
        }

        for target in routes.targets() {
            if !guests.contains_key(target) {
                bail!("route targets guest `{target}`, which is not registered");
            }
        }

        tracing::info!(guests = guests.len(), "runtime initialized");

        Ok(Self {
            engine,
            options,
            guests,
            routes,
            dispatch,
        })
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
    pub fn get(&self, id: &GuestId) -> Option<&Guest<T>> {
        self.guests.get(id)
    }

    /// Iterate every registered guest in a deterministic, identity-sorted order
    /// so per-trigger capability and ambiguity errors are stable across runs.
    ///
    /// The order falls out of the [`BTreeMap`] keying; no per-call sort.
    pub fn guests(&self) -> impl Iterator<Item = &Guest<T>> {
        self.guests.values()
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
        self.guests.len()
    }

    /// Returns `true` if the registry has no guests (never, post-construction).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.guests.is_empty()
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

    #[test]
    fn no_guests() {
        let options = RuntimeOptions::load().expect("options should load");
        let engine = Engine::new(&Config::from(&options)).expect("engine should build");
        let linker = Linker::<StoreCtx<()>>::new(&engine);
        let dispatch = DispatchHandle::new(
            Arc::new(FirstArgSelector),
            BTreeSet::new(),
            8,
            std::time::Duration::from_secs(30),
        );

        let result =
            Registry::assemble(engine, linker, options, Vec::new(), Routes::default(), dispatch);
        assert!(result.is_err(), "an empty registry must be rejected");
    }
}
