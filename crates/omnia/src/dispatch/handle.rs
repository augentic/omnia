//! Shared dispatch state: selector, link allow-list, transport, and depth bound.

use std::collections::BTreeSet;
use std::sync::{Arc, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};
use std::time::Duration;

use anyhow::{Result, bail};

use super::selector::GuestSelector;
use super::transport::InProcess;
use crate::registry::GuestId;

tokio::task_local! {
    // The context of the dispatch chain the current task is serving. Carried
    // across the in-process carrier via the wRPC accept context and
    // re-established around each served invocation, so concurrent, unrelated
    // chains never share a depth budget or a wall-clock policy.
    static CHAIN_CTX: ChainCtx;
}

/// Per-chain dispatch context: the nesting depth (0 at a chain root) plus
/// whether the chain root runs uncapped (a command-mode `wasi:cli/run` drive).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ChainCtx {
    pub(super) depth: usize,
    pub(super) uncapped: bool,
}

/// Run `fut` with the chain context carried over from an incoming dispatch, so
/// nested host-mediated calls made while serving it count against the same
/// chain and inherit its wall-clock policy.
pub(super) fn with_chain<F>(ctx: ChainCtx, fut: F) -> impl Future<Output = F::Output>
where
    F: Future,
{
    CHAIN_CTX.scope(ctx, fut)
}

/// Run `fut` as a command-mode chain root: link dispatches it makes (and their
/// nested hops) run without the `GUEST_TIMEOUT_MS` wall-clock cap.
pub fn as_command_chain<F>(fut: F) -> impl Future<Output = F::Output>
where
    F: Future,
{
    CHAIN_CTX.scope(
        ChainCtx {
            depth: 0,
            uncapped: true,
        },
        fut,
    )
}

/// The context of the dispatch chain currently being served (a capped root
/// outside any scope).
fn current_chain() -> ChainCtx {
    CHAIN_CTX.try_with(|ctx| *ctx).unwrap_or_default()
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
    /// the context the dispatched call runs at (depth plus the inherited
    /// wall-clock policy), to be carried to the serve side.
    ///
    /// Depth is per call chain (A->B->C, each awaited to completion before the
    /// caller returns), so concurrent, unrelated chains never contend for the
    /// same budget.
    pub(super) fn enter(&self, target: &GuestId) -> Result<ChainCtx> {
        let current = current_chain();
        let depth = current.depth + 1;
        if depth > self.max_depth {
            bail!(
                "link dispatch depth {depth} exceeds maximum {} (target `{target}`); raise \
                 MAX_DISPATCH_DEPTH if this is intentional",
                self.max_depth
            );
        }
        Ok(ChainCtx {
            depth,
            uncapped: current.uncapped,
        })
    }
}
