//! How a trigger or the command driver runs a guest: it takes a slot of the
//! concurrency gate ([`Runtime::slot`] or [`Runtime::try_slot`]), then enters
//! the guest on a fresh store with it, a trigger through [`Runtime::enter`].
//! The [`Entered`] guard that comes back holds the store and the slot.

use std::sync::Arc;

use anyhow::Result;
use wasmtime::component::Instance;
use wasmtime::{AsContext, AsContextMut, Store, StoreContext, StoreContextMut};

use super::Runtime;
use super::gate::{Invocations, Slot};
use crate::chain::ChainCtx;
use crate::host::Backends;
use crate::invoke::explain_full_pool;
use crate::registry::Guest;
use crate::store::StoreCtx;
use crate::telemetry;

impl<B: Backends> Runtime<B> {
    /// Instantiate `guest` on a fresh store at `chain`, admitted by `slot`:
    /// how a trigger runs a guest.
    ///
    /// The store is built for `guest` alone and lives in the returned
    /// [`Entered`], beside the guest's [`Instance`] (instance-per-call). Its
    /// guest sees no arguments at any chain context, [`ChainCtx::command`]
    /// included. A command's run receives them through
    /// [`run_command`](Self::run_command). `slot` comes from
    /// [`slot`](Self::slot) or [`try_slot`](Self::try_slot), and frees once
    /// the store has dropped.
    ///
    /// # Errors
    ///
    /// Returns an error if the component cannot be instantiated, such as
    /// when the instance pool is full.
    ///
    /// # Panics
    ///
    /// Panics if `slot` was taken from another runtime's gate, or if
    /// `MAX_FUEL` is set but the engine was built without fuel metering.
    pub async fn enter(
        &self, guest: &Guest<B>, chain: ChainCtx, slot: Slot,
    ) -> Result<(Entered<B>, Instance)> {
        self.instantiate(guest, self.store_ctx(guest, chain, None), slot).await
    }

    // the only store given the command's arguments
    pub(super) async fn enter_command(
        &self, guest: &Guest<B>, slot: Slot,
    ) -> Result<(Entered<B>, Instance)> {
        let args = Some(Arc::clone(&self.inner.args));
        self.instantiate(guest, self.store_ctx(guest, ChainCtx::command(), args), slot).await
    }

    // the guard owns the store from the start, so a failed or cancelled
    // instantiation drops the store before the slot
    async fn instantiate(
        &self, guest: &Guest<B>, data: StoreCtx<B>, slot: Slot,
    ) -> Result<(Entered<B>, Instance)> {
        assert!(self.inner.gate.issued(&slot), "the slot was taken from another runtime's gate");
        let mut entered = Entered {
            store: self.build_store(data),
            _slot: slot,
        };
        let instance = guest
            .instance_pre()
            .instantiate_async(&mut entered.store)
            .await
            .map_err(explain_full_pool)?;
        tracing::debug!(guest = %guest.id(), "component instantiated");
        Ok((entered, instance))
    }

    /// Takes a slot of the concurrency gate, waiting as long as it takes for
    /// one to free.
    ///
    /// A root invocation, a trigger's or the command driver's, passes the
    /// slot to [`enter`](Self::enter), which holds it while the store lives.
    /// A link hop or a dispatch from inside a store takes none, since its
    /// root holds one. Waiters are served in the order they arrive.
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

/// A guest entered on a fresh store, holding the slot of the concurrency gate
/// that admitted it.
///
/// [`Runtime::enter`] returns one beside the guest's [`Instance`]. It
/// implements [`AsContextMut`], so `&mut entered` passes wherever wasmtime
/// takes a store. Dropping it drops the store first, then frees the slot.
#[must_use = "dropping it ends the invocation"]
pub struct Entered<B: 'static> {
    // fields drop in declaration order, so the pool has the store's instance
    // back before the slot admits the next invocation
    store: Store<StoreCtx<B>>,
    _slot: Slot,
}

impl<B: 'static> AsContext for Entered<B> {
    type Data = StoreCtx<B>;

    fn as_context(&self) -> StoreContext<'_, StoreCtx<B>> {
        self.store.as_context()
    }
}

impl<B: 'static> AsContextMut for Entered<B> {
    fn as_context_mut(&mut self) -> StoreContextMut<'_, StoreCtx<B>> {
        self.store.as_context_mut()
    }
}
