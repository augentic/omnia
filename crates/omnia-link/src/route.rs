//! # Route table
//!
//! Host-mediated calls are routed in memory: the caller's lifted [`Val`]s are
//! handed to a fresh callee instance driven on its own task by
//! [`omnia_core::call_fresh`], and the callee's results move back the same
//! way. [`RouteInvoke`] is the seam a remote target would implement later by
//! serialising the `Vec<Val>`; nothing in the dispatch path depends on the
//! callee being co-located.

use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::sync::{Arc, PoisonError, RwLock};
use std::time::Duration;

use anyhow::{Result, anyhow, bail};
use futures::FutureExt as _;
use futures::future::ready;
use omnia_core::{ChainCtx, FreshCall, FutureResult, GuestId, StoreFactory, call_fresh};
use wasmtime::component::{ComponentExportIndex, InstancePre, Val};

/// Delivers one call to a resolved target.
pub trait RouteInvoke: Send + Sync {
    /// Invoke `interface`/`func` on the target with `args` at `ctx` in its
    /// dispatch chain, bounded by `bound` when given.
    fn invoke(
        &self, interface: &str, func: &str, args: Vec<Val>, ctx: ChainCtx, bound: Option<Duration>,
    ) -> FutureResult<Vec<Val>>;
}

/// One linked export, resolved once on the component rather than per call.
pub struct Linked {
    pub export: ComponentExportIndex,
    pub results: usize,
}

/// A served guest's linked exports, keyed by interface then function, plus
/// what instantiating it fresh per call needs.
pub struct Route<T: 'static> {
    pub factory: StoreFactory<T>,
    pub instance_pre: InstancePre<T>,
    pub funcs: HashMap<Box<str>, HashMap<Box<str>, Linked>>,
}

impl<T: Send + 'static> RouteInvoke for Route<T> {
    fn invoke(
        &self, interface: &str, func: &str, args: Vec<Val>, ctx: ChainCtx, bound: Option<Duration>,
    ) -> FutureResult<Vec<Val>> {
        let Some(linked) = self.funcs.get(interface).and_then(|funcs| funcs.get(func)) else {
            return ready(Err(anyhow!("guest exports `{interface}` but no `{func}`"))).boxed();
        };
        let call = FreshCall {
            factory: Arc::clone(&self.factory),
            instance_pre: self.instance_pre.clone(),
            export: linked.export,
            results: linked.results,
        };
        // `anyhow::Error::from` keeps the `InvokeError` downcastable so the
        // polyfill can word a timeout with the target and function.
        async move { call_fresh(call, args, ctx, bound).await.map_err(anyhow::Error::from) }.boxed()
    }
}

/// The live route table every polyfilled import resolves its target from.
///
/// Routes move through two stages. Serving a guest *parks* its route — `None`
/// when it exports no linked interface — as pending, outside the registry's
/// lifecycle gate; publishing moves it to the live map under that gate,
/// together with the registry entry, so the two change as one step.
/// [`resolve`](Self::resolve) reads only the live map, under the map's own
/// lock: a call racing a deregister may complete against the departing
/// instance, exactly as an in-flight invocation does.
///
/// Both maps are `Arc`-shared so a guest registered after bootstrap is
/// reachable from every clone of the table (serve-at-register).
#[derive(Clone, Default)]
pub struct Routes {
    inner: Arc<RwLock<RouteTable>>,
}

/// Recording every served guest — linked or not — lets `resolve` tell "not
/// registered" from "registered but exports nothing linked".
#[derive(Default)]
struct RouteTable {
    pending: HashMap<GuestId, Option<Arc<dyn RouteInvoke>>>,
    live: HashMap<GuestId, Option<Arc<dyn RouteInvoke>>>,
}

impl Routes {
    /// Park `target`'s served route as pending until it is published or
    /// discarded, refusing an identity that is already pending.
    ///
    /// Runs outside the registry's lifecycle gate; takes only the map lock.
    pub fn park(&self, target: &GuestId, route: Option<Arc<dyn RouteInvoke>>) -> Result<()> {
        let mut table = self.inner.write().unwrap_or_else(PoisonError::into_inner);
        match table.pending.entry(target.clone()) {
            Entry::Occupied(_) => bail!("guest `{target}` already has a pending route"),
            Entry::Vacant(slot) => {
                slot.insert(route);
                Ok(())
            }
        }
    }

    /// Move `target`'s pending route live, refusing an occupied live slot so a
    /// registration can never clobber an existing guest's route; a no-op when
    /// nothing is pending. A refused pending route is dropped.
    ///
    /// The caller must hold the registry's lifecycle write guard.
    pub fn publish(&self, target: &GuestId) -> Result<()> {
        let mut table = self.inner.write().unwrap_or_else(PoisonError::into_inner);
        let Some(route) = table.pending.remove(target) else {
            return Ok(());
        };
        match table.live.entry(target.clone()) {
            Entry::Occupied(_) => bail!("guest `{target}` already has a live route"),
            Entry::Vacant(slot) => {
                slot.insert(route);
                Ok(())
            }
        }
    }

    /// Drop `target`'s pending route (a publication that was refused).
    ///
    /// The caller must hold the registry's lifecycle write guard.
    pub fn discard(&self, target: &GuestId) {
        self.inner.write().unwrap_or_else(PoisonError::into_inner).pending.remove(target);
    }

    /// Drop `target`'s live route; in-flight invocations hold their own
    /// [`Arc`] and complete.
    ///
    /// The caller must hold the registry's lifecycle write guard.
    pub fn remove(&self, target: &GuestId) {
        self.inner.write().unwrap_or_else(PoisonError::into_inner).live.remove(target);
    }

    /// Drop every pending and live route, so a finished deployment releases
    /// the `Runtime` clones (and the engine) their store factories pin.
    pub fn clear(&self) {
        let mut table = self.inner.write().unwrap_or_else(PoisonError::into_inner);
        table.pending.clear();
        table.live.clear();
    }

    /// The live route for `target`, for a call to `interface`, or `None`
    /// when `target` is not registered.
    ///
    /// # Errors
    ///
    /// Returns an error if `target` is registered but exports no linked
    /// interface.
    pub fn lookup(
        &self, target: &GuestId, interface: &str,
    ) -> Result<Option<Arc<dyn RouteInvoke>>> {
        let table = self.inner.read().unwrap_or_else(PoisonError::into_inner);
        match table.live.get(target) {
            None => Ok(None),
            Some(None) => bail!(
                "guest `{target}` is registered but exports no linked interface (`{interface}`); \
                 is it meant to be a link target?"
            ),
            Some(Some(route)) => Ok(Some(Arc::clone(route))),
        }
    }

    /// The live route for `target`, for a call to `interface`.
    ///
    /// # Errors
    ///
    /// Returns an error if `target` is not registered, or is registered but
    /// exports no linked interface.
    pub fn resolve(&self, target: &GuestId, interface: &str) -> Result<Arc<dyn RouteInvoke>> {
        self.lookup(target, interface)?.ok_or_else(|| anyhow!("guest `{target}` is not registered"))
    }
}
