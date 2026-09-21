//! Per-chain dispatch context and the policy that bounds it.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Result, bail};

use crate::RuntimeOptions;
use crate::registry::GuestId;

tokio::task_local! {
    // The context of the dispatch chain the current task is serving,
    // re-established around each spawned callee task, so concurrent, unrelated
    // chains never share a depth budget, a wall-clock policy, or baggage. A
    // hop may replace the baggage it hands on, hence the cell.
    static CHAIN_CTX: RefCell<ChainCtx>;
}

/// Per-chain dispatch context: nesting depth, wall-clock policy, and the
/// baggage carried to each hop dispatched beneath.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChainCtx {
    /// Nesting depth of the current hop (0 at a chain root).
    pub depth: usize,
    /// Whether the chain root runs without the wall-clock cap.
    pub uncapped: bool,
    // Name/value entries the chain carries, opaque here: a capability sets
    // them through `set_baggage` and every hop beneath reads them through
    // `baggage`. Shared, since every `enter` snapshots the context.
    baggage: Arc<BTreeMap<String, String>>,
}

impl ChainCtx {
    /// The root of a command-mode chain: a `wasi:cli/run` drive, whose link
    /// dispatches (and their nested hops) run without the `GUEST_TIMEOUT_MS`
    /// wall-clock cap.
    #[must_use]
    pub fn command() -> Self {
        Self::root(true)
    }

    /// The root of a server chain: a trigger-served guest, whose link
    /// dispatches (and their nested hops) run under the wall-clock cap.
    #[must_use]
    pub fn server() -> Self {
        Self::root(false)
    }

    fn root(uncapped: bool) -> Self {
        Self {
            depth: 0,
            uncapped,
            baggage: Arc::default(),
        }
    }
}

/// Scopes a future to a dispatch chain, as `tracing::Instrument` scopes one to
/// a span.
pub trait Chained: Future + Sized {
    /// Run `self` at `ctx` in its dispatch chain, so nested host-mediated
    /// calls made while it runs count against that chain and inherit its
    /// wall-clock policy and baggage.
    fn in_chain(self, ctx: ChainCtx) -> impl Future<Output = Self::Output>;
}

impl<F: Future> Chained for F {
    fn in_chain(self, ctx: ChainCtx) -> impl Future<Output = Self::Output> {
        CHAIN_CTX.scope(RefCell::new(ctx), self)
    }
}

/// The baggage of the chain the current task serves; empty outside any chain.
#[must_use]
pub fn baggage() -> Arc<BTreeMap<String, String>> {
    CHAIN_CTX.try_with(|ctx| Arc::clone(&ctx.borrow().baggage)).unwrap_or_default()
}

/// Replace the baggage of the chain the current task serves, for the hops
/// dispatched beneath it from now on.
///
/// A hop already dispatched keeps the snapshot it was given. A no-op outside
/// any chain.
pub fn set_baggage(baggage: BTreeMap<String, String>) {
    let _ = CHAIN_CTX.try_with(|ctx| ctx.borrow_mut().baggage = Arc::new(baggage));
}

/// The deployment-wide bounds on a dispatch chain: its maximum nesting depth
/// and the wall-clock cap on each server-rooted hop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChainPolicy {
    /// Maximum dispatch nesting depth (`MAX_DISPATCH_DEPTH`).
    pub max_depth: usize,
    /// Wall-clock bound on each host-mediated dispatch (`GUEST_TIMEOUT_MS`).
    pub timeout: Duration,
}

impl ChainPolicy {
    /// Enter a dispatch, bounding the current chain's nesting depth; returns
    /// the context the dispatched call runs at (depth plus the inherited
    /// wall-clock policy and baggage), to be carried to the serve side.
    ///
    /// Depth is per call chain (A->B->C, each awaited to completion before the
    /// caller returns), so concurrent, unrelated chains never contend for the
    /// same budget. The returned context is a snapshot: baggage the caller
    /// sets after entering never reaches this hop.
    ///
    /// # Errors
    ///
    /// Returns an error if the hop would exceed `max_depth`.
    pub fn enter(&self, target: &GuestId) -> Result<ChainCtx> {
        let current =
            CHAIN_CTX.try_with(|ctx| ctx.borrow().clone()).unwrap_or_else(|_| ChainCtx::server());
        let depth = current.depth + 1;

        if depth > self.max_depth {
            bail!(
                "link dispatch depth {depth} exceeds maximum {} (target `{target}`); raise \
                 MAX_DISPATCH_DEPTH if this is intentional",
                self.max_depth
            );
        }

        Ok(ChainCtx { depth, ..current })
    }
}

impl From<&RuntimeOptions> for ChainPolicy {
    fn from(options: &RuntimeOptions) -> Self {
        Self {
            max_depth: options.max_dispatch_depth,
            timeout: options.guest_timeout,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy() -> ChainPolicy {
        ChainPolicy {
            max_depth: 4,
            timeout: Duration::from_secs(1),
        }
    }

    fn callee() -> GuestId {
        GuestId::from("callee")
    }

    fn entries(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs.iter().map(|(name, value)| ((*name).to_owned(), (*value).to_owned())).collect()
    }

    // Baggage set at a hop reaches both the context `enter` hands the callee
    // and the callee's own reads, one and two hops down without resetting.
    #[tokio::test]
    async fn inherited() {
        async {
            set_baggage(entries(&[("k", "v")]));
            let child = policy().enter(&callee()).expect("within depth");
            assert_eq!(*child.baggage, entries(&[("k", "v")]));
            assert_eq!((child.depth, child.uncapped), (1, true));

            async {
                assert_eq!(*baggage(), entries(&[("k", "v")]));
                let grandchild = policy().enter(&callee()).expect("within depth");
                assert_eq!(*grandchild.baggage, entries(&[("k", "v")]));
                assert_eq!(grandchild.depth, 2);
            }
            .in_chain(child)
            .await;
        }
        .in_chain(ChainCtx::command())
        .await;
    }

    // A callee's context is a snapshot taken at `enter`: what its caller or a
    // sibling sets afterwards never reaches it, and what it sets never
    // reaches them.
    #[tokio::test]
    async fn snapshot() {
        async {
            set_baggage(entries(&[("k", "first")]));
            let first = policy().enter(&callee()).expect("within depth");
            set_baggage(entries(&[("k", "second")]));
            let second = policy().enter(&callee()).expect("within depth");
            assert_eq!(*first.baggage, entries(&[("k", "first")]));
            assert_eq!(*second.baggage, entries(&[("k", "second")]));

            async {
                set_baggage(entries(&[("k", "child")]));
                assert_eq!(*baggage(), entries(&[("k", "child")]));
            }
            .in_chain(first)
            .await;
            assert_eq!(*baggage(), entries(&[("k", "second")]));
        }
        .in_chain(ChainCtx::command())
        .await;
    }

    // A server root is capped at depth 0 and carries baggage like a command
    // root.
    #[tokio::test]
    async fn server_rooted() {
        async {
            set_baggage(entries(&[("k", "v")]));
            let child = policy().enter(&callee()).expect("within depth");
            assert_eq!((child.depth, child.uncapped), (1, false));
            assert_eq!(*child.baggage, entries(&[("k", "v")]));
        }
        .in_chain(ChainCtx::server())
        .await;
    }

    // Outside any chain there is nothing to carry baggage on: the write is
    // inert and a hop entered there roots a capped chain with none.
    #[tokio::test]
    async fn unscoped() {
        set_baggage(entries(&[("k", "v")]));
        assert!(baggage().is_empty());
        let child = policy().enter(&callee()).expect("within depth");
        assert!(child.baggage.is_empty());
        assert_eq!((child.depth, child.uncapped), (1, false));
    }
}
