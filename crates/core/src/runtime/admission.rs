//! The admission guards: one first-use load in flight and one seat at a time
//! per guest identity.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, PoisonError};

use futures::future::{BoxFuture, FutureExt as _, Shared, WeakShared};
use tokio::sync::Notify;

use crate::registry::GuestId;

type Load<T> = BoxFuture<'static, T>;

// The first-use loads in flight, one per identity. Each is held weakly, so it
// runs while any caller still awaits it and is dropped with the last one.
pub(super) struct Loads<T> {
    state: Arc<Mutex<State<T>>>,
}

struct State<T> {
    next: u64,
    running: HashMap<GuestId, Running<T>>,
}

struct Running<T> {
    token: u64,
    load: WeakShared<Load<T>>,
}

impl<T: Clone + Send + Sync + 'static> Loads<T> {
    // a strong handle is never dropped under the lock: the last one drops the
    // load, and its `Loading` takes the lock
    pub(super) fn join(&self, id: &GuestId, start: impl FnOnce() -> Load<T>) -> Shared<Load<T>> {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(load) = state.running.get(id).and_then(|running| running.load.upgrade()) {
            return load;
        }

        let token = state.next;
        state.next += 1;
        let loading = Loading {
            state: Arc::clone(&self.state),
            id: id.clone(),
            token,
        };
        let body = start();
        let load = async move {
            let _loading = loading;
            body.await
        }
        .boxed()
        .shared();
        let weak = load.downgrade().expect("a load not yet polled has not completed");
        state.running.insert(id.clone(), Running { token, load: weak });
        load
    }
}

impl<T> Default for Loads<T> {
    fn default() -> Self {
        Self {
            state: Arc::new(Mutex::new(State {
                next: 0,
                running: HashMap::new(),
            })),
        }
    }
}

// Removes its load's entry however the load ends: finished, or dropped by its
// last caller. A newer load can take the entry of one its callers abandoned
// before this runs, so only the entry under its own token goes.
struct Loading<T> {
    state: Arc<Mutex<State<T>>>,
    id: GuestId,
    token: u64,
}

impl<T> Drop for Loading<T> {
    fn drop(&mut self) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if state.running.get(&self.id).is_some_and(|running| running.token == self.token) {
            state.running.remove(&self.id);
        }
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
