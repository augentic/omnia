//! Shared dispatch state: selector, link allow-list, transport, and depth bound.

use std::collections::BTreeSet;
use std::sync::{Arc, OnceLock, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};
use std::time::Duration;

use anyhow::{Result, bail};

use super::resolve::ResolveHook;
use super::selector::GuestSelector;
use super::transport::InProcess;
use crate::registry::GuestId;

tokio::task_local! {
    // The nesting depth of the dispatch chain the current task is serving
    // (0 at a chain root). Carried across the in-process carrier via the
    // wRPC accept context and re-established around each served invocation,
    // so concurrent, unrelated chains never share a depth budget.
    static CHAIN_DEPTH: usize;
}

/// Run `fut` with the chain depth carried over from an incoming dispatch, so
/// nested host-mediated calls made while serving it count against the same
/// chain.
pub(super) fn with_depth<F>(depth: usize, fut: F) -> impl Future<Output = F::Output>
where
    F: Future,
{
    CHAIN_DEPTH.scope(depth, fut)
}

/// The chain depth of the dispatch currently being served (0 at a chain root).
fn current_depth() -> usize {
    CHAIN_DEPTH.try_with(|depth| *depth).unwrap_or(0)
}

/// The long-lived dispatch state shared by every polyfilled import.
///
/// It carries the selector strategy, the union of host-mediated interfaces, the
/// bound transport carrier, the guest-lifecycle gate, the per-dispatch
/// wall-clock bound, and the per-chain dispatch-depth bound.
pub struct DispatchHandle {
    pub(super) selector: Arc<dyn GuestSelector>,
    links: BTreeSet<Box<str>>,
    transport: InProcess,
    // Serializes guest lifecycle transitions (register/deregister/bootstrap
    // serve wiring) against readers, so the registry map and the transport's
    // endpoint map always change as one atomic step. Lock order: this gate
    // first, then a single inner map — never the other way around, and never
    // across an await.
    lifecycle: Arc<RwLock<()>>,
    // Resolve-on-miss hook: lets the link path fault a missing target in
    // through the runtime's resolver (RFC guest-resolution §4.5). Installed
    // once, when the deployment carries a resolver.
    resolve_hook: OnceLock<Box<dyn ResolveHook>>,
    max_depth: usize,
    timeout: Duration,
}

impl DispatchHandle {
    /// Create a shared dispatch handle. The transport carrier starts empty;
    /// [`super::serve_links`] (via [`crate::Runtime::new`]) populates it with
    /// each target's serve side.
    #[must_use]
    pub fn new(
        selector: Arc<dyn GuestSelector>, links: BTreeSet<Box<str>>, max_depth: usize,
        timeout: Duration,
    ) -> Arc<Self> {
        let lifecycle = Arc::new(RwLock::new(()));
        Arc::new(Self {
            selector,
            links,
            transport: InProcess::new(Arc::clone(&lifecycle)),
            lifecycle,
            resolve_hook: OnceLock::new(),
            max_depth,
            timeout,
        })
    }

    /// Wall-clock bound applied to each host-mediated dispatch (the
    /// deployment's `guest_timeout`).
    #[must_use]
    pub(super) const fn timeout(&self) -> Duration {
        self.timeout
    }

    /// The union of host-mediated interface names across every guest's `link`
    /// allow-list — the set of interfaces to polyfill (caller side) and serve
    /// (callee side).
    #[must_use]
    pub const fn links(&self) -> &BTreeSet<Box<str>> {
        &self.links
    }

    /// The bound transport carrier.
    pub(crate) const fn transport(&self) -> &InProcess {
        &self.transport
    }

    /// Install the resolve-on-miss hook; a second install is ignored (the
    /// hook is deployment-scoped, set once during runtime assembly).
    pub(crate) fn set_resolve_hook(&self, hook: Box<dyn ResolveHook>) {
        if self.resolve_hook.set(hook).is_err() {
            tracing::warn!("resolve hook already installed; ignoring");
        }
    }

    /// The resolve-on-miss hook, if a resolver is installed.
    pub(crate) fn resolve_hook(&self) -> Option<&dyn ResolveHook> {
        self.resolve_hook.get().map(Box::as_ref)
    }

    /// Enter a lifecycle read section: registry/transport lookups taken under
    /// this guard never observe a half-applied register or deregister.
    pub(crate) fn lifecycle_read(&self) -> RwLockReadGuard<'_, ()> {
        self.lifecycle.read().unwrap_or_else(PoisonError::into_inner)
    }

    /// Enter a lifecycle write section: the holder may mutate the registry
    /// map and the transport endpoint map as one atomic transition.
    pub(crate) fn lifecycle_write(&self) -> RwLockWriteGuard<'_, ()> {
        self.lifecycle.write().unwrap_or_else(PoisonError::into_inner)
    }

    /// Enter a dispatch, bounding the current chain's nesting depth; returns
    /// the depth the dispatched call runs at, to be carried to the serve side.
    ///
    /// Depth is per call chain (A->B->C, each awaited to completion before the
    /// caller returns), so concurrent, unrelated chains never contend for the
    /// same budget.
    pub(super) fn enter(&self, target: &GuestId) -> Result<usize> {
        let depth = current_depth() + 1;
        if depth > self.max_depth {
            bail!(
                "link dispatch depth {depth} exceeds maximum {} (target `{target}`); raise \
                 MAX_DISPATCH_DEPTH if this is intentional",
                self.max_depth
            );
        }
        Ok(depth)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::{CHAIN_DEPTH, DispatchHandle};
    use crate::dispatch::FirstArgSelector;
    use crate::registry::GuestId;

    fn handle(max_depth: usize) -> Arc<DispatchHandle> {
        DispatchHandle::new(
            Arc::new(FirstArgSelector),
            std::iter::empty().collect(),
            max_depth,
            std::time::Duration::from_secs(30),
        )
    }

    #[test]
    fn depth_bound() {
        let handle = handle(2);
        let target = GuestId::from("t");

        // A chain root enters at depth 1; a serve at depth 1 enters at 2.
        assert_eq!(handle.enter(&target).expect("root within bound"), 1);
        CHAIN_DEPTH.sync_scope(1, || {
            assert_eq!(handle.enter(&target).expect("depth 2 within bound"), 2);
        });
        CHAIN_DEPTH.sync_scope(2, || {
            handle.enter(&target).expect_err("depth 3 exceeds the maximum");
        });
    }
}
