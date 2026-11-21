//! The concurrency gate: how many invocations a runtime runs at once.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

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

    // A slot, waiting at most `wait` for one to free, or as long as it takes
    // when `wait` is `None`. A zero wait takes a free slot or none.
    pub(super) async fn take(&self, wait: Option<Duration>) -> Option<Slot> {
        match self.slots.try_acquire() {
            Ok(permit) => permit.forget(),
            Err(_) if wait.is_some_and(|wait| wait.is_zero()) => return None,
            Err(_) => {
                let _waiting = Waiting::enter(&self.waiting);
                let acquired = match wait {
                    None => self.slots.acquire().await.ok(),
                    Some(wait) => tokio::time::timeout(wait, self.slots.acquire())
                        .await
                        .ok()
                        .and_then(Result::ok),
                };
                acquired?.forget();
            }
        }
        Some(Slot(Arc::clone(&self.slots)))
    }

    pub(super) fn occupancy(&self) -> Invocations {
        Invocations {
            in_flight: self.size - self.slots.available_permits(),
            waiting: self.waiting.load(Ordering::Relaxed),
            limit: self.size,
        }
    }
}

// A held slot, freed on drop as a permit is. A tokio permit is
// `#[clippy::has_significant_drop]`, which a store holding one would inherit,
// flagging every site downstream that keeps a store alive.
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
pub struct Invocations {
    /// The invocations holding a slot.
    pub in_flight: usize,
    /// The invocations waiting for a slot to free.
    pub waiting: usize,
    /// The slots the gate holds: `MAX_CONCURRENT_INVOCATIONS`, or the size
    /// the deployment resolved for it.
    pub limit: usize,
}
