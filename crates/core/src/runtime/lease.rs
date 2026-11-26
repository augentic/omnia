//! How a root invocation is admitted and run: a [`Lease`] holds its slot of
//! the concurrency gate and its deadline, and [`Lease::invoke`] enters the
//! guest and runs a call on it before that deadline.

use std::future::Future;
use std::pin::pin;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use futures::future::OptionFuture;
use tokio::sync::watch;
use tokio::task::coop::unconstrained;
use tokio::time::{Instant, sleep};
use wasmtime::PoolConcurrencyLimitError;
use wasmtime::component::Instance;

use super::Runtime;
use super::enter::Entered;
use super::gate::{Invocations, Slot};
use crate::host::Backends;
use crate::invoke::explain_full_pool;
use crate::registry::Guest;
use crate::telemetry;

impl<B: Backends> Runtime<B> {
    /// Admit an invocation of `guest`, waiting as long as it takes for a slot to free.
    ///
    /// The invocation's deadline is the
    /// [guest timeout](crate::RuntimeOptions::guest_timeout), counted from
    /// the moment the slot is taken. Waiters are served in the order they
    /// arrive, and a freed slot goes to one before
    /// [`try_lease`](Self::try_lease) sees it.
    pub async fn lease(&self, guest: Arc<Guest<B>>) -> Lease<B> {
        let slot = self.slot().await;
        self.server_lease(guest, slot)
    }

    /// Admit an invocation of `guest` if a slot is free, or return `None` at once.
    ///
    /// A refusal counts on the `invocations_refused` counter. The deadline
    /// of a lease taken here counts from now, as for [`lease`](Self::lease).
    #[must_use]
    pub fn try_lease(&self, guest: Arc<Guest<B>>) -> Option<Lease<B>> {
        let slot = self.try_slot()?;
        Some(self.server_lease(guest, slot))
    }

    // the command run's, uncapped like the chain it roots, and the only lease
    // whose store is given the command's arguments
    pub(super) async fn command_lease(&self, guest: Arc<Guest<B>>) -> Lease<B> {
        let slot = self.slot().await;
        Lease {
            runtime: self.clone(),
            guest,
            slot,
            root: Root::Command,
        }
    }

    fn server_lease(&self, guest: Arc<Guest<B>>, slot: Slot) -> Lease<B> {
        let limit = self.options().guest_timeout;
        Lease {
            runtime: self.clone(),
            guest,
            slot,
            root: Root::Server {
                arm: Arm::now(limit),
                millis: epoch_millis(limit),
            },
        }
    }

    pub(crate) async fn slot(&self) -> Slot {
        self.inner.gate.take().await
    }

    pub(crate) fn try_slot(&self) -> Option<Slot> {
        let slot = self.inner.gate.try_take();
        if slot.is_none() {
            telemetry::count_refused(self.name());
        }
        slot
    }

    /// How full the concurrency gate is now.
    #[must_use]
    pub fn invocations(&self) -> Invocations {
        self.inner.gate.occupancy()
    }
}

/// An admitted invocation of one guest, holding its slot of the concurrency gate and its deadline.
///
/// [`Runtime::lease`] and [`Runtime::try_lease`] take one. Its slot frees
/// once [`invoke`](Self::invoke) has dropped the guest's store, or when the
/// lease drops unused.
#[must_use = "a lease admits an invocation only while it is held"]
pub struct Lease<B: 'static> {
    runtime: Runtime<B>,
    guest: Arc<Guest<B>>,
    slot: Slot,
    root: Root,
}

impl<B: Backends> Lease<B> {
    /// The instant the invocation's deadline expires, in milliseconds since the Unix epoch.
    ///
    /// It is the deadline [`invoke`](Self::invoke) enforces, for a trigger to
    /// tell its guest. `None` when no deadline bounds the invocation.
    #[must_use]
    pub const fn deadline_millis(&self) -> Option<u64> {
        match self.root {
            Root::Server { millis, .. } => Some(millis),
            Root::Command => None,
        }
    }

    /// Instantiate the guest on a fresh store and run `call` on it, before the deadline.
    ///
    /// `call` receives:
    /// - the [`Entered`] store, which holds the lease's slot from then on;
    /// - the guest's [`Instance`];
    /// - the invocation's [`Bound`], to re-arm the deadline.
    ///
    /// The deadline bounds instantiation and the call alike. Once it passes,
    /// `call`'s future is dropped with the store, and then the slot frees.
    /// The guest sees no arguments, which only [`Runtime::run_command`]
    /// passes.
    ///
    /// # Errors
    ///
    /// - [`LeaseError::Enter`] when the guest cannot be instantiated.
    /// - [`LeaseError::PoolFull`] when the instance pool has no room for the
    ///   guest's instance.
    /// - [`LeaseError::Timeout`] when the deadline passes before `call`
    ///   returns.
    ///
    /// # Panics
    ///
    /// Panics if `MAX_FUEL` is set but the engine was built without fuel
    /// metering.
    pub async fn invoke<F, Fut, R>(self, call: F) -> Result<R, LeaseError>
    where
        F: FnOnce(Entered<B>, Instance, Bound) -> Fut + Send,
        Fut: Future<Output = R> + Send,
        R: Send,
    {
        let Self {
            runtime,
            guest,
            slot,
            root,
        } = self;
        let (bound, mut armed) = Bound::new(root.arm());
        let run = async move {
            let entered = match root {
                Root::Server { .. } => runtime.enter(&guest, slot).await,
                Root::Command => runtime.enter_command(&guest, slot).await,
            };
            let (entered, instance) = entered?;
            Ok(call(entered, instance, bound).await)
        };
        let mut run = pin!(run);

        // the timer races the whole invocation from outside it, since wasmtime
        // does not reliably fire a timer raced inside `run_concurrent`
        loop {
            let expiry = (*armed.borrow_and_update()).map(Arm::expiry);

            // a call that returns as the deadline passes has returned in time,
            // and one that spends the task's coop budget cannot starve the timer
            tokio::select! {
                biased;
                outcome = &mut run => return outcome,
                Ok(()) = unconstrained(armed.changed()) => {}
                Some(limit) = unconstrained(OptionFuture::from(expiry)) => {
                    return Err(LeaseError::Timeout(limit));
                }
            }
        }
    }
}

#[derive(Clone, Copy)]
enum Root {
    // a trigger's, at a server chain root, due by `arm` and told as `millis`
    Server { arm: Arm, millis: u64 },
    Command,
}

impl Root {
    const fn arm(self) -> Option<Arm> {
        match self {
            Self::Server { arm, .. } => Some(arm),
            Self::Command => None,
        }
    }
}

#[derive(Clone, Copy)]
struct Arm {
    from: Instant,
    limit: Duration,
}

impl Arm {
    fn now(limit: Duration) -> Self {
        Self {
            from: Instant::now(),
            limit,
        }
    }

    // a sleep of the time left, which tokio saturates for any limit
    async fn expiry(self) -> Duration {
        sleep(self.limit.saturating_sub(self.from.elapsed())).await;
        self.limit
    }
}

// saturates rather than panics, so no configured timeout can fail an invocation
fn epoch_millis(after: Duration) -> u64 {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    u64::try_from(now.saturating_add(after).as_millis()).unwrap_or(u64::MAX)
}

/// The deadline of a leased invocation, which the call it runs can re-arm.
///
/// [`Lease::invoke`] hands one to its call. A call re-arms it when the work
/// it has left runs to a limit of its own, such as a response body that
/// streams on after its head.
pub struct Bound(watch::Sender<Option<Arm>>);

impl Bound {
    fn new(arm: Option<Arm>) -> (Self, watch::Receiver<Option<Arm>>) {
        let (sender, armed) = watch::channel(arm);
        (Self(sender), armed)
    }

    /// Re-arm the invocation's deadline to `limit` from now, or lift it with `None`.
    ///
    /// The deadline armed before stops counting at once. A
    /// [`LeaseError::Timeout`] that follows names `limit`.
    pub fn reset(&self, limit: Option<Duration>) {
        self.0.send_replace(limit.map(Arm::now));
    }
}

/// Why a leased invocation did not return its call's output.
#[derive(Debug, thiserror::Error)]
pub enum LeaseError {
    /// The guest could not be instantiated.
    #[error(transparent)]
    Enter(anyhow::Error),
    /// The instance pool had no room for the guest's instance.
    ///
    /// The concurrency gate admitted the invocation, but the pool had no free
    /// slot of a kind the instance takes. A retry can succeed once another
    /// invocation ends.
    #[error(transparent)]
    PoolFull(anyhow::Error),
    /// The deadline passed before the call returned.
    ///
    /// It carries the limit the deadline was armed with last.
    #[error("timed out after {0:?}")]
    Timeout(Duration),
}

impl LeaseError {
    pub(super) fn from_instantiation(error: wasmtime::Error) -> Self {
        let full = error.is::<PoolConcurrencyLimitError>();
        let error = explain_full_pool(error);
        if full { Self::PoolFull(error) } else { Self::Enter(error) }
    }
}
