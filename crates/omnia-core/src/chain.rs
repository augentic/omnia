//! Per-chain dispatch context and the policy that bounds it.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Result, bail};

use crate::RuntimeOptions;
use crate::registry::GuestId;

/// Per-chain dispatch context: nesting depth, wall-clock policy, and the
/// metadata carried to each hop dispatched beneath.
///
/// Every guest store is built at a context (see
/// [`StoreBase::chain`](crate::StoreBase::chain)): a trigger or the command
/// driver builds the root at [`server`](Self::server) or
/// [`command`](Self::command), and a link dispatch builds the callee at the
/// snapshot [`ChainPolicy::enter`] derives from the caller's.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChainCtx {
    /// Nesting depth of the current hop (0 at a chain root).
    pub depth: usize,
    /// Whether the chain root runs without the wall-clock cap.
    pub uncapped: bool,
    // Name/value entries the chain carries, opaque here: a capability's host
    // binding writes them (`omnia:otel/baggage` shows them to a guest as its
    // baggage) and every hop beneath reads them. Shared, since every `enter`
    // snapshots the context.
    metadata: Arc<BTreeMap<String, String>>,
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
            metadata: Arc::default(),
        }
    }

    /// The name/value metadata this hop was dispatched with, plus what it has
    /// set since.
    #[must_use]
    pub fn metadata(&self) -> &BTreeMap<String, String> {
        &self.metadata
    }

    /// The metadata, for a host binding that extends it for the hops
    /// dispatched beneath this one from now on; a hop already dispatched
    /// keeps the snapshot it was given.
    pub fn metadata_mut(&mut self) -> &mut BTreeMap<String, String> {
        Arc::make_mut(&mut self.metadata)
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
    /// inherited wall-clock policy and metadata), to be carried to the serve
    /// side.
    ///
    /// Depth is per call chain (A->B->C, each awaited to completion before the
    /// caller returns), so concurrent, unrelated chains never contend for the
    /// same budget. The returned context is a snapshot: metadata the caller
    /// sets after entering never reaches this hop.
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

        Ok(ChainCtx {
            depth,
            ..caller.clone()
        })
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

    fn entries(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs.iter().map(|(name, value)| ((*name).to_owned(), (*value).to_owned())).collect()
    }

    // Metadata set at a hop reaches the context `enter` hands the callee, one
    // and two hops down, with the depth counting up and the root's wall-clock
    // policy carried; what a hop sets afterwards stays out of the snapshot.
    #[test]
    fn inherited() {
        let mut root = ChainCtx::command();
        root.metadata_mut().extend(entries(&[("k", "v"), ("tenant", "acme")]));

        let child = policy().enter(&root, &callee()).expect("within depth");
        assert_eq!((child.depth, child.uncapped), (1, true));
        assert_eq!(*child.metadata(), entries(&[("k", "v"), ("tenant", "acme")]));

        root.metadata_mut().extend(entries(&[("k", "later")]));
        assert_eq!(*root.metadata(), entries(&[("k", "later"), ("tenant", "acme")]));
        assert_eq!(*child.metadata(), entries(&[("k", "v"), ("tenant", "acme")]));

        let grandchild = policy().enter(&child, &callee()).expect("within depth");
        assert_eq!((grandchild.depth, grandchild.uncapped), (2, true));
        assert_eq!(*grandchild.metadata(), entries(&[("k", "v"), ("tenant", "acme")]));
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
