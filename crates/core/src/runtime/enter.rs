//! The one way a trigger or the command driver runs a guest:
//! [`Runtime::enter`].

use std::time::Duration;

use wasmtime::component::Instance;
use wasmtime::{PoolConcurrencyLimitError, Store};

use super::Runtime;
use crate::chain::ChainCtx;
use crate::host::Backends;
use crate::registry::Guest;
use crate::store::StoreCtx;
use crate::telemetry;

impl<B: Backends> Runtime<B> {
    /// Instantiate `guest` on a fresh store at `chain` that holds one slot
    /// of the concurrency gate: how a trigger or the command driver runs a
    /// guest.
    ///
    /// The store is the one [`store_factory`](Self::store_factory) builds for
    /// `guest`, and drops with the invocation (instance-per-call). The slot
    /// frees as the store drops, however long the caller keeps it.
    ///
    /// When every slot is taken, `enter` waits for one to free: at most
    /// `wait`, or as long as it takes when `wait` is `None`. A zero `wait`
    /// takes a free slot or none. Each refusal counts on the
    /// `invocations_refused` metric.
    ///
    /// # Errors
    ///
    /// - [`EnterError::Busy`] if no slot freed within `wait`, or the instance
    ///   pool is full.
    /// - [`EnterError::Failed`] if the component cannot be instantiated.
    ///
    /// # Panics
    ///
    /// Panics if `MAX_FUEL` is set but the engine was built without fuel
    /// metering.
    pub async fn enter(
        &self, guest: &Guest<B>, chain: ChainCtx, wait: Option<Duration>,
    ) -> Result<(Store<StoreCtx<B>>, Instance), EnterError> {
        let Some(slot) = self.inner.gate.take(wait).await else {
            telemetry::count_refused();
            return Err(EnterError::Busy);
        };

        let mut store = self.build_store(self.store_ctx(guest, chain));
        store.data_mut().base.slot = Some(slot);
        let instance = match guest.instance_pre().instantiate_async(&mut store).await {
            Ok(instance) => instance,
            Err(error) if error.is::<PoolConcurrencyLimitError>() => {
                tracing::warn!(guest = %guest.id(), %error, "instance pool is full");
                telemetry::count_refused();
                return Err(EnterError::Busy);
            }
            Err(error) => return Err(EnterError::Failed(error.into())),
        };
        tracing::debug!(guest = %guest.id(), "component instantiated");
        Ok((store, instance))
    }
}

/// Why [`Runtime::enter`] produced no instance.
#[derive(Debug, thiserror::Error)]
pub enum EnterError {
    /// No slot freed within `wait`, or the instance pool is full.
    ///
    /// The runtime is at capacity rather than broken, so a caller answers
    /// it as a transport's "try again later".
    #[error("the runtime is at capacity: no invocation slot is free, or the instance pool is full")]
    Busy,
    /// The guest could not be instantiated.
    #[error(transparent)]
    Failed(anyhow::Error),
}
