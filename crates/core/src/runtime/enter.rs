//! How a trigger or the command driver runs a guest: it takes a slot of the
//! concurrency gate ([`Runtime::slot`] or [`Runtime::try_slot`]), then
//! [`Runtime::enter`]s the guest.

use anyhow::Result;
use wasmtime::Store;
use wasmtime::component::Instance;

use super::Runtime;
use super::gate::{Invocations, Slot};
use crate::chain::ChainCtx;
use crate::host::Backends;
use crate::invoke::explain_full_pool;
use crate::registry::Guest;
use crate::store::StoreCtx;
use crate::telemetry;

impl<B: Backends> Runtime<B> {
    /// Instantiate `guest` on a fresh store at `chain`: how a trigger or the
    /// command driver runs a guest.
    ///
    /// The store is built for `guest` alone and drops with the invocation
    /// (instance-per-call). `enter` takes no slot of the concurrency gate. A
    /// root invocation takes one first, through [`slot`](Self::slot) or
    /// [`try_slot`](Self::try_slot), and holds it until the store has
    /// dropped.
    ///
    /// # Errors
    ///
    /// Returns an error if the component cannot be instantiated, such as
    /// when the instance pool is full.
    ///
    /// # Panics
    ///
    /// Panics if `MAX_FUEL` is set but the engine was built without fuel
    /// metering.
    pub async fn enter(
        &self, guest: &Guest<B>, chain: ChainCtx,
    ) -> Result<(Store<StoreCtx<B>>, Instance)> {
        let mut store = self.build_store(self.store_ctx(guest, chain));
        let instance =
            guest.instance_pre().instantiate_async(&mut store).await.map_err(explain_full_pool)?;
        tracing::debug!(guest = %guest.id(), "component instantiated");
        Ok((store, instance))
    }

    /// Takes a slot of the concurrency gate, waiting as long as it takes for
    /// one to free.
    ///
    /// A root invocation, a trigger's or the command driver's, holds a slot
    /// while its store lives. A link hop or a host dispatch takes none,
    /// since its root holds one. Waiters are served in the order they
    /// arrive. Declare the slot before the store, so the store drops first.
    pub async fn slot(&self) -> Slot {
        self.inner.gate.take().await
    }

    /// Takes a free slot of the concurrency gate, or returns `None` at once
    /// when every slot is held.
    ///
    /// A refusal counts on the `invocations_refused` counter. A slot taken
    /// here is held as for [`slot`](Self::slot).
    #[must_use]
    pub fn try_slot(&self) -> Option<Slot> {
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
