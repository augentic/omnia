//! Shared dispatch state: selector, link allow-list, transport, and depth bound.

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use anyhow::{Context as _, Result, bail};

use super::selector::GuestSelector;
use super::transport::InProcess;
use crate::registry::GuestId;

/// The long-lived dispatch state shared by every polyfilled import.
///
/// It carries the selector strategy, the union of host-mediated interfaces, the
/// bound transport (installed once the serve side is wired), the per-dispatch
/// wall-clock bound, and the process-wide dispatch-depth counter.
pub struct DispatchHandle {
    pub(super) selector: Arc<dyn GuestSelector>,
    links: BTreeSet<Box<str>>,
    transport: OnceLock<InProcess>,
    depth: AtomicUsize,
    max_depth: usize,
    timeout: Duration,
}

impl DispatchHandle {
    /// Create a shared dispatch handle. The transport is installed later by
    /// [`super::serve_links`], once each target's serve side is wired.
    #[must_use]
    pub fn new(
        selector: Arc<dyn GuestSelector>, links: BTreeSet<Box<str>>, max_depth: usize,
        timeout: Duration,
    ) -> Arc<Self> {
        Arc::new(Self {
            selector,
            links,
            transport: OnceLock::new(),
            depth: AtomicUsize::new(0),
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

    /// Install the bound transport carrier (called once by [`super::serve_links`]).
    pub(super) fn install(&self, transport: InProcess) {
        // Set-once: a second install (there is only ever one) is ignored.
        let _ = self.transport.set(transport);
    }

    /// The bound transport carrier.
    pub(crate) fn transport(&self) -> Result<&InProcess> {
        self.transport
            .get()
            .context("link transport not initialized; `serve_links` must run before dispatch")
    }

    /// Enter a dispatch, bounding nesting depth. The returned guard decrements
    /// the shared counter on drop.
    ///
    /// The counter is process-wide and tracks *synchronous* nesting (A->B->C,
    /// each awaited to completion before the caller returns); it is a safety
    /// bound, not a precise per-chain limit under heavy concurrency.
    pub(super) fn enter(&self, target: &GuestId) -> Result<DepthGuard<'_>> {
        let depth = self.depth.fetch_add(1, Ordering::SeqCst) + 1;
        if depth > self.max_depth {
            self.depth.fetch_sub(1, Ordering::SeqCst);
            bail!(
                "link dispatch depth {depth} exceeds maximum {} (target `{target}`); raise \
                 MAX_DISPATCH_DEPTH if this is intentional",
                self.max_depth
            );
        }
        Ok(DepthGuard { depth: &self.depth })
    }
}

/// Decrements the shared dispatch-depth counter when a dispatch unwinds.
#[derive(Debug)]
pub(super) struct DepthGuard<'a> {
    depth: &'a AtomicUsize,
}

impl Drop for DepthGuard<'_> {
    fn drop(&mut self) {
        self.depth.fetch_sub(1, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::Ordering;

    use super::DispatchHandle;
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
    fn depth_guard() {
        let handle = handle(2);
        let target = GuestId::from("t");

        let first = handle.enter(&target).expect("depth 1 within bound");
        let second = handle.enter(&target).expect("depth 2 within bound");
        handle.enter(&target).expect_err("depth 3 exceeds the maximum");

        // Unwinding the guards frees the budget again.
        drop(second);
        drop(first);
        assert_eq!(handle.depth.load(Ordering::SeqCst), 0);
        handle.enter(&target).expect("budget freed after guards drop");
    }
}
