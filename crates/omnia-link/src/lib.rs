//! # Host-mediated dynamic linking
//!
//! A caller guest imports an interface (say `example:link/echo`) whose
//! implementation the host satisfies at runtime. The host polyfills that import
//! on the shared `Linker` so invoking it:
//!
//! 1. extracts a target identity from the call via a [`GuestSelector`],
//! 2. rejects any resource handle attempting to cross the seam,
//! 3. enforces a dispatch-depth bound,
//! 4. resolves the target from the live route table and instantiates it
//!    *fresh* on a new store, driving the matching export on its own task
//!    through `omnia_core::call_fresh`, and
//! 5. moves the typed results back into the caller, discarding the callee
//!    instance.
//!
//! Because step 4 is always a fresh instance, a dispatched call cannot
//! recursively re-enter its caller. The runtime core stays generic: nothing
//! declares the seam. Each component says what it imports and what it
//! exports; every import outside the runtime's own namespaces
//! ([`is_host`]) is relayed and every export outside them is routed, by
//! opaque string, resolving opaque [`GuestId`]s — the seam never parses a
//! consumer scheme. Arguments and results travel as wasmtime
//! [`Val`](wasmtime::component::Val)s lifted from the caller and lowered into
//! the callee, with no codec in between; the selector runs in the polyfill on
//! those lifted values, so it sees typed parameters. Sync-typed functions are
//! registered with `func_new_async`; async-typed (`async func`) ones with
//! `func_new_concurrent`; both share one body whose only read of the caller's
//! store is a snapshot of its chain context, which the callee's store is
//! built from. See `docs/Architecture.md` (The Guest Registry) for the full
//! design.
//!
//! [`InProcessLinks`] is the [`LinkSeam`] the registry drives when omnia is
//! built with its `link` feature.

#![cfg(not(target_arch = "wasm32"))]

mod polyfill;
mod route;
mod selector;
mod serve;

use std::sync::{Arc, Mutex, PoisonError};

use anyhow::Result;
use futures::FutureExt as _;
use futures::future::ready;
use omnia_core::{
    ChainPolicy, FutureResult, Guest, GuestId, HasChain, HasDispatcher, LinkSeam, LoadedGuest,
    StoreFactory,
};
use wasmtime::Engine;
use wasmtime::component::{Component, Linker};

use self::polyfill::{Caller, WiredLinks};
use self::route::Routes;
pub use self::selector::{FirstArgSelector, GuestSelector};

/// The runtime's own namespaces.
///
/// An import under one is a host capability, linked before any guest; an
/// export under one is a trigger the host drives. Everything else crosses
/// between guests: the seam relays the import and routes the export.
#[must_use]
pub fn is_host(interface: &str) -> bool {
    ["wasi:", "omnia:"].iter().any(|namespace| interface.starts_with(namespace))
}

/// Guest→guest linking by in-memory routing to a fresh callee instance.
///
/// Holds the selector strategy, the chain policy (depth and wall-clock
/// bounds), the route table, and the functions the bootstrap polyfilled onto
/// the shared linker.
pub struct InProcessLinks {
    selector: Arc<dyn GuestSelector>,
    policy: ChainPolicy,
    routes: Routes,
    // the bootstrap wiring; a late guest polyfills on a linker clone against a copy
    wired: Mutex<WiredLinks>,
}

impl InProcessLinks {
    /// Create the seam under `policy`. The route table starts empty; the
    /// registry serves and publishes each guest's route through the
    /// [`LinkSeam`] methods.
    #[must_use]
    pub fn new(selector: Arc<dyn GuestSelector>, policy: ChainPolicy) -> Self {
        Self {
            selector,
            policy,
            routes: Routes::default(),
            wired: Mutex::new(WiredLinks::new()),
        }
    }

    // What every polyfilled import captures for its per-call dispatch.
    fn caller(&self) -> Arc<Caller> {
        Arc::new(Caller {
            selector: Arc::clone(&self.selector),
            policy: self.policy,
            routes: self.routes.clone(),
        })
    }
}

impl<T: HasChain + HasDispatcher + 'static> LinkSeam<T> for InProcessLinks {
    fn polyfill(
        &self, engine: &Engine, linker: &mut Linker<T>, guests: &[LoadedGuest],
    ) -> Result<()> {
        let caller = self.caller();
        let mut wired = WiredLinks::new();
        for LoadedGuest { id, component, .. } in guests {
            polyfill::polyfill_component(engine, linker, id, component, &caller, &mut wired)?;
        }
        *self.wired.lock().unwrap_or_else(PoisonError::into_inner) = wired;
        Ok(())
    }

    fn polyfill_late(
        &self, engine: &Engine, linker: &mut Linker<T>, id: &GuestId, component: &Component,
    ) -> Result<()> {
        // a copy, so a linker clone's wiring never leaks into the bootstrap record
        let mut wired = self.wired.lock().unwrap_or_else(PoisonError::into_inner).clone();
        polyfill::polyfill_component(engine, linker, id, component, &self.caller(), &mut wired)
    }

    fn serve(&self, factory: StoreFactory<T>, guest: &Guest<T>) -> FutureResult<()> {
        // pure introspection: the route is built here, the future only carries its outcome
        let wired = self.wired.lock().unwrap_or_else(PoisonError::into_inner).clone();
        let parked = serve::serve_guest(
            &self.routes,
            &wired,
            factory,
            guest.id(),
            guest.instance_pre().clone(),
        );
        ready(parked).boxed()
    }

    fn publish(&self, id: &GuestId) -> Result<()> {
        self.routes.publish(id)
    }

    fn discard(&self, id: &GuestId) {
        self.routes.discard(id);
    }

    fn remove(&self, id: &GuestId) {
        self.routes.remove(id);
    }

    fn shutdown(&self) {
        self.routes.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::is_host;

    #[test]
    fn host_namespaces() {
        for interface in ["wasi:cli/run@0.2.0", "wasi:http/types", "omnia:plugins/loader@0.1.0"] {
            assert!(is_host(interface), "{interface}");
        }
    }

    #[test]
    fn guest_namespaces() {
        for interface in ["omnia-test:link/ops", "emery:adapter/source@0.1.0", "acme:ledger/ops"] {
            assert!(!is_host(interface), "{interface}");
        }
    }
}
