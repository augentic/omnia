//! The admission guards: one first-use load in flight and one seat at a time
//! per guest identity.

use std::collections::{HashMap, HashSet};
use std::panic::AssertUnwindSafe;
use std::sync::{Arc, Mutex, PoisonError};

use futures::future::{BoxFuture, FutureExt as _, Shared, WeakShared};
use tokio::sync::Notify;

use super::GuestError;
use crate::registry::{Guest, GuestId};

type Loaded<B> = Result<Arc<Guest<B>>, GuestError>;
type Load<B> = BoxFuture<'static, Loaded<B>>;

// The first-use loads in flight, one per declared guest. Each is held weakly,
// so it runs while any caller still awaits it and is dropped with the last
// one. No entry is removed: a deployment declares a fixed set of guests, and
// a weak entry keeps neither a guest nor the runtime alive.
pub(super) struct Loads<B: 'static>(Mutex<HashMap<GuestId, WeakShared<Load<B>>>>);

impl<B: 'static> Loads<B> {
    // A load that has ended is never joined, so the first use after a failure
    // loads afresh. A panic in `load` ends it as an error each waiter takes:
    // left to unwind, it would poison the load and leave a waiter asleep.
    pub(super) fn join(
        &self, id: &GuestId, load: impl Future<Output = Loaded<B>> + Send + 'static,
    ) -> Shared<Load<B>> {
        let mut loads = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let running = loads.get(id).and_then(WeakShared::upgrade);
        if let Some(running) = running.filter(|running| running.peek().is_none()) {
            return running;
        }

        let panicked = GuestError::Internal(format!("loading guest `{id}` panicked"));
        let load = AssertUnwindSafe(load)
            .catch_unwind()
            .map(|caught| caught.unwrap_or(Err(panicked)))
            .boxed()
            .shared();
        let weak = load.downgrade().expect("a load not yet polled has not completed");
        loads.insert(id.clone(), weak);
        load
    }
}

impl<B: 'static> Default for Loads<B> {
    fn default() -> Self {
        Self(Mutex::default())
    }
}

// The identities being seated, one seat each at a time. A seat that races
// another waits for it to end, then finds the identity occupied, or free when
// the other failed.
#[derive(Default)]
pub(super) struct Seats {
    held: Mutex<HashSet<GuestId>>,
    freed: Notify,
}

impl Seats {
    // the wait is armed before each check, so a seat freed between the check
    // and the wait still wakes it
    pub(super) async fn hold(&self, id: &GuestId) -> Seating<'_> {
        loop {
            let freed = self.freed.notified();
            let taken = self.held.lock().unwrap_or_else(PoisonError::into_inner).insert(id.clone());
            if taken {
                return Seating {
                    seats: self,
                    id: id.clone(),
                };
            }
            freed.await;
        }
    }
}

// holds an identity's seat for as long as its seating runs, however it ends
pub(super) struct Seating<'a> {
    seats: &'a Seats,
    id: GuestId,
}

impl Drop for Seating<'_> {
    fn drop(&mut self) {
        self.seats.held.lock().unwrap_or_else(PoisonError::into_inner).remove(&self.id);
        self.seats.freed.notify_waiters();
    }
}
