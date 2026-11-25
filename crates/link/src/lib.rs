//! # Host-mediated dynamic linking
//!
//! A caller guest imports an interface (say `example:link/echo`) whose
//! implementation the host satisfies at runtime. The host polyfills that import
//! on the shared `Linker` so invoking it:
//!
//! 1. reads the target identity from the call's leading string argument,
//! 2. refuses a target outside the caller's
//!    [`guests`](omnia_core::Grants::guests) grant, before the target loads,
//! 3. rejects any resource handle attempting to cross the seam,
//! 4. enforces a dispatch-depth bound, before the target loads,
//! 5. calls the target's export of the same name, matched exactly, on a
//!    *fresh* instance on its own task, through the calling store's
//!    [`Dispatcher`](omnia_core::Dispatcher), and
//! 6. moves the typed results back into the caller, discarding the callee
//!    instance.
//!
//! Because step 5 is always a fresh instance, a dispatched call cannot
//! recursively re-enter its caller. The runtime core stays generic: nothing
//! declares the seam. Each component says what it imports and what it
//! exports; every import outside the runtime's own namespaces
//! ([`is_host`]) is relayed, by opaque string, resolving opaque
//! [`GuestId`]s — the seam never parses a consumer scheme. Arguments and
//! results travel as wasmtime [`Val`](wasmtime::component::Val)s lifted from
//! the caller and lowered into the callee, with no codec in between. The
//! leading argument names the target and is forwarded too, since caller and
//! callee share one interface. A call whose leading argument is not a string
//! is refused. Sync-typed functions are registered with `func_new_async`;
//! async-typed (`async func`) ones with `func_new_concurrent`; both share one
//! body whose only read of the caller's store is a snapshot of its identity,
//! grants, chain context, and dispatcher. The callee's store is built from
//! that chain context. See `docs/Architecture.md` (The Guest Registry) for the
//! full design.
//!
//! [`InProcessLinks`] is the `LinkSeam` the registry drives when omnia is
//! built with its `link` feature.

#![cfg(not(target_arch = "wasm32"))]

mod polyfill;
mod signature;

use std::sync::{Mutex, PoisonError};

use anyhow::Result;
use omnia_core::{Guest, GuestId, LinkSeam, LoadedGuest, StoreCtx};
use wasmtime::Engine;
use wasmtime::component::{Component, Linker};

use self::polyfill::WiredLinks;

/// The runtime's own namespaces.
///
/// An import under one is a host capability, linked before any guest; an
/// export under one is a trigger the host drives. Everything else crosses
/// between guests: the seam relays the import to the target's export of the
/// same name.
#[must_use]
pub fn is_host(interface: &str) -> bool {
    ["wasi:", "omnia:"].iter().any(|namespace| interface.starts_with(namespace))
}

/// Guest→guest linking through the calling store's
/// [`Dispatcher`](omnia_core::Dispatcher), to a fresh callee instance.
///
/// Holds the functions the bootstrap polyfilled onto the shared linker.
#[derive(Default)]
pub struct InProcessLinks {
    // the bootstrap wiring; a late guest polyfills on a linker clone against a copy
    wired: Mutex<WiredLinks>,
}

impl InProcessLinks {
    /// Create the seam, with nothing polyfilled yet.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

impl<B: Send + 'static> LinkSeam<B> for InProcessLinks {
    fn polyfill(
        &self, engine: &Engine, linker: &mut Linker<StoreCtx<B>>, guests: &[LoadedGuest],
    ) -> Result<()> {
        let mut wired = WiredLinks::new();
        for LoadedGuest { id, component, .. } in guests {
            polyfill::polyfill_component(engine, linker, id, component, &mut wired)?;
        }
        *self.wired.lock().unwrap_or_else(PoisonError::into_inner) = wired;
        Ok(())
    }

    fn polyfill_late(
        &self, engine: &Engine, linker: &mut Linker<StoreCtx<B>>, id: &GuestId,
        component: &Component,
    ) -> Result<()> {
        // a copy, so a linker clone's wiring never leaks into the bootstrap record
        let mut wired = self.wired.lock().unwrap_or_else(PoisonError::into_inner).clone();
        polyfill::polyfill_component(engine, linker, id, component, &mut wired)
    }

    fn check(&self, guest: &Guest<B>, standing: &[&Guest<B>]) -> Result<()> {
        signature::check(guest, standing)
    }
}
