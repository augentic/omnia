//! The concurrency gate: how many invocations a runtime runs at once.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use tokio::sync::Semaphore;

// One slot per invocation a trigger or the command driver runs. A link hop
// or a host dispatch takes none, since the root it serves holds one and
// would deadlock a full gate waiting on its own hop.
pub(super) struct Gate {
    slots: Arc<Semaphore>,
    size: usize,
    waiting: AtomicUsize,
}

impl Gate {
    pub(super) fn new(size: usize) -> Self {
        let size = size.clamp(1, Semaphore::MAX_PERMITS);
        Self {
            slots: Arc::new(Semaphore::new(size)),
            size,
            waiting: AtomicUsize::new(0),
        }
    }

    // tokio's semaphore is fair, so waiters are served in the order they
    // arrive, and a freed slot goes to a waiter before `try_take` sees it
    pub(super) async fn take(&self) -> Slot {
        if let Some(slot) = self.try_take() {
            return slot;
        }
        let _waiting = Waiting::enter(&self.waiting);
        self.slots.acquire().await.expect("the gate is never closed").forget();
        Slot(Arc::clone(&self.slots))
    }

    pub(super) fn try_take(&self) -> Option<Slot> {
        self.slots.try_acquire().ok()?.forget();
        Some(Slot(Arc::clone(&self.slots)))
    }

    pub(super) fn issued(&self, slot: &Slot) -> bool {
        Arc::ptr_eq(&self.slots, &slot.0)
    }

    pub(super) fn occupancy(&self) -> Invocations {
        Invocations {
            in_flight: self.size - self.slots.available_permits(),
            waiting: self.waiting.load(Ordering::Relaxed),
            limit: self.size,
        }
    }
}

/// One slot of the concurrency gate, held by a root invocation and freed on drop.
///
/// [`Runtime::slot`](super::Runtime::slot) and
/// [`Runtime::try_slot`](super::Runtime::try_slot) take one, and
/// [`Runtime::enter`](super::Runtime::enter) holds it in the
/// [`Entered`](super::Entered) it returns, so it frees only after the
/// invocation's store has dropped. A slot admits an invocation only on the
/// runtime whose gate issued it.
// not a tokio permit, whose `#[clippy::has_significant_drop]` would flag
// every site that holds a slot for as long as its store
#[derive(Debug)]
#[must_use = "a slot admits an invocation only while it is held"]
pub struct Slot(Arc<Semaphore>);

impl Drop for Slot {
    fn drop(&mut self) {
        self.0.add_permits(1);
    }
}

// counts a waiter for as long as it waits, however its wait ends
struct Waiting<'a>(&'a AtomicUsize);

impl<'a> Waiting<'a> {
    fn enter(waiting: &'a AtomicUsize) -> Self {
        waiting.fetch_add(1, Ordering::Relaxed);
        Self(waiting)
    }
}

impl Drop for Waiting<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Relaxed);
    }
}

/// How full the concurrency gate is at one moment.
///
/// [`Runtime::invocations`](super::Runtime::invocations) takes the snapshot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Invocations {
    /// The invocations holding a slot.
    pub in_flight: usize,
    /// The invocations waiting for a slot to free.
    pub waiting: usize,
    /// The slots the gate holds: `MAX_CONCURRENT_INVOCATIONS`, or
    /// `POOL_MAX_INSTANCES` when it is unset, at least one.
    pub limit: usize,
}
