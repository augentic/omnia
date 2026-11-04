//! Per-chain dispatch context and the policy that bounds it.

use std::time::Duration;

use anyhow::{Result, bail};

use crate::RuntimeOptions;
use crate::registry::GuestId;

/// Per-chain dispatch context: nesting depth and the wall-clock policy carried
/// to each hop dispatched beneath.
///
/// Every guest store is built at a context (see
/// [`StoreBase::chain`](crate::StoreBase::chain)): a trigger or the command
/// driver builds the root at [`server`](Self::server) or
/// [`command`](Self::command), and a link dispatch builds the callee at the
/// context [`ChainPolicy::enter`] derives from the caller's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChainCtx {
    /// Nesting depth of the current hop (0 at a chain root).
    pub depth: usize,
    /// Whether the chain root runs without the wall-clock cap.
    pub uncapped: bool,
}

impl ChainCtx {
    /// The root of a command-mode chain: a `wasi:cli/run` drive, whose link
    /// dispatches (and their nested hops) run without the `GUEST_TIMEOUT_MS`
    /// wall-clock cap.
    #[must_use]
    pub const fn command() -> Self {
        Self::root(true)
    }

    /// The root of a server chain: a trigger-served guest, whose link
    /// dispatches (and their nested hops) run under the wall-clock cap.
    #[must_use]
    pub const fn server() -> Self {
        Self::root(false)
    }

    const fn root(uncapped: bool) -> Self {
        Self { depth: 0, uncapped }
    }
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
    /// Enter a dispatch from `caller`, bounding the chain's nesting depth;
    /// returns the context the dispatched call runs at (depth plus the
    /// inherited wall-clock policy), to be carried to the serve side.
    ///
    /// Depth is per call chain (A->B->C, each awaited to completion before the
    /// caller returns), so concurrent, unrelated chains never contend for the
    /// same budget.
    ///
    /// # Errors
    ///
    /// Returns an error if the hop would exceed `max_depth`.
    pub fn enter(&self, caller: &ChainCtx, target: &GuestId) -> Result<ChainCtx> {
        let depth = caller.depth + 1;

        if depth > self.max_depth {
            bail!(
                "link dispatch depth {depth} exceeds maximum {} (target `{target}`); raise \
                 MAX_DISPATCH_DEPTH if this is intentional",
                self.max_depth
            );
        }

        Ok(ChainCtx { depth, ..*caller })
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
            max_depth: 2,
            timeout: Duration::from_secs(1),
        }
    }

    fn callee() -> GuestId {
        GuestId::from("callee")
    }

    // The depth counts up one and two hops down from a command root, with the
    // root's wall-clock policy carried to each.
    #[test]
    fn inherited() {
        let root = ChainCtx::command();

        let child = policy().enter(&root, &callee()).expect("within depth");
        assert_eq!((child.depth, child.uncapped), (1, true));

        let grandchild = policy().enter(&child, &callee()).expect("within depth");
        assert_eq!((grandchild.depth, grandchild.uncapped), (2, true));
    }

    // A server root's hops run capped, and the hop past the bound is refused
    // naming the target and the option that raises it.
    #[test]
    fn over_depth() {
        let root = ChainCtx::server();
        let child = policy().enter(&root, &callee()).expect("within depth");
        assert!(!child.uncapped);
        let grandchild = policy().enter(&child, &callee()).expect("at the bound");

        let error = policy().enter(&grandchild, &callee()).expect_err("over the bound");
        let text = error.to_string();
        for needle in ["depth 3", "maximum 2", "`callee`", "MAX_DISPATCH_DEPTH"] {
            assert!(text.contains(needle), "`{needle}` missing from: {text}");
        }
    }
}
