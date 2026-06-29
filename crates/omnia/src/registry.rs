//! # Guest registry
//!
//! One [`Engine`] and one `Linker` hold many pre-instantiated guests at once,
//! each selectable by an opaque [`GuestId`]. A registry entry is instantiated
//! fresh per call and discarded (instance-per-call). This is pure wasmtime
//! infrastructure: it is what lets one process route an HTTP request, a CLI
//! command, and a topic message to *different* guests.
//!
//! The floor treats identities as opaque keys; consumers project their own
//! scheme onto them. Omnia never parses a [`GuestId`].

mod routing;

pub use routing::{CliRoutes, HttpRoutes, Resolver, Routes, TopicRoutes, TriggerRouter};

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use anyhow::{Context as _, Result, bail};
use wasmtime::Engine;
use wasmtime::component::{Component, InstancePre, Linker};
use wasmtime_wasi::WasiView;
use wrpc_wasmtime::WrpcView;

use crate::RuntimeOptions;
use crate::deployment::LoadedGuest;
use crate::dispatch::{self, DispatchHandle};

/// Opaque guest identity.
///
/// The floor treats it as an ordered string key; consumers (e.g. Specify)
/// project their own scheme onto it (`source:typescript`, ...). Omnia never
/// parses it.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct GuestId(pub Arc<str>);

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
/// Phase 1 only populates [`Target::Local`]. `Remote` (a bound wRPC endpoint)
/// arrives with the cluster transports and is what makes the desktop->cloud
/// swap a config change rather than a code change.
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

/// Assembles a [`Registry`] from loaded guest components and a fully linked
/// linker.
///
/// Pre-instantiation, route validation, and registry construction happen in
/// [`build`](Self::build). [`Deployment::build`](crate::Deployment::build) is
/// the usual entry point.
pub struct RegistryBuilder<T: WasiView + 'static> {
    engine: Engine,
    linker: Linker<T>,
    options: RuntimeOptions,
    loaded: Vec<LoadedGuest>,
    routes: Routes,
    dispatch: Arc<DispatchHandle>,
}

impl<T: WasiView + 'static> RegistryBuilder<T> {
    pub const fn new(
        engine: Engine, linker: Linker<T>, options: RuntimeOptions, loaded: Vec<LoadedGuest>,
        routes: Routes, dispatch: Arc<DispatchHandle>,
    ) -> Self {
        Self {
            engine,
            linker,
            options,
            loaded,
            routes,
            dispatch,
        }
    }

    /// Polyfill host-mediated imports, pre-instantiate every loaded guest, and
    /// freeze the guest [`Registry`].
    ///
    /// # Errors
    ///
    /// Returns an error if there are no guests to register, host-mediated
    /// imports cannot be polyfilled, a component cannot be pre-instantiated, or
    /// a route targets a guest that is not registered.
    pub fn build(self) -> Result<Registry<T>>
    where
        T: WrpcView,
    {
        if self.loaded.is_empty() {
            bail!("cannot build a guest registry with no guests");
        }

        let mut linker = self.linker;
        dispatch::link(&self.engine, &mut linker, &self.loaded, &self.dispatch)?;

        let mut guests = BTreeMap::new();
        for loaded in &self.loaded {
            let instance_pre = linker
                .instantiate_pre(&loaded.component)
                .map_err(anyhow::Error::from)
                .with_context(|| format!("pre-instantiating guest `{}`", loaded.id))?;
            guests.insert(loaded.id.clone(), Guest::local(loaded.id.clone(), instance_pre));
        }

        for target in self.routes.targets() {
            if !guests.contains_key(target) {
                bail!("route targets guest `{target}`, which is not registered");
            }
        }

        tracing::info!(guests = guests.len(), "runtime initialized");

        Ok(Registry {
            engine: self.engine,
            options: self.options,
            guests,
            routes: self.routes,
            dispatch: self.dispatch,
        })
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
    fn guest_id() {
        let id = GuestId::from("source:typescript");
        assert_eq!(id.as_str(), "source:typescript");
        assert_eq!(id.to_string(), "source:typescript");
        assert_eq!(GuestId::from(String::from("workflow")), GuestId::from("workflow"));
        assert!(GuestId::from("a") < GuestId::from("b"));
    }

    #[test]
    fn no_guests() {
        let options = RuntimeOptions::load().expect("options should load");
        let engine = Engine::new(&Config::from(&options)).expect("engine should build");
        let linker = Linker::<StoreCtx<()>>::new(&engine);
        let dispatch = DispatchHandle::new(Arc::new(FirstArgSelector), BTreeSet::new(), 8);

        let result =
            RegistryBuilder::new(engine, linker, options, Vec::new(), Routes::default(), dispatch)
                .build();
        assert!(result.is_err(), "an empty registry must be rejected");
    }
}
