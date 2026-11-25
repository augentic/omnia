//! How a root invocation enters its guest: on a fresh store, which an
//! [`Entered`] guard holds together with the invocation's slot of the
//! concurrency gate.

use std::sync::Arc;

use anyhow::Result;
use wasmtime::component::Instance;
use wasmtime::{AsContext, AsContextMut, Store, StoreContext, StoreContextMut};

use super::Runtime;
use super::gate::Slot;
use crate::chain::ChainCtx;
use crate::host::Backends;
use crate::invoke::explain_full_pool;
use crate::registry::Guest;
use crate::store::StoreCtx;

impl<B: Backends> Runtime<B> {
    // a server chain root's store, whose guest sees no arguments
    pub(super) async fn enter(
        &self, guest: &Guest<B>, slot: Slot,
    ) -> Result<(Entered<B>, Instance)> {
        self.instantiate(guest, self.store_ctx(guest, ChainCtx::server(), None), slot).await
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
}

/// A guest entered on a fresh store, holding its invocation's slot of the concurrency gate.
///
/// [`Lease::invoke`](super::Lease::invoke) hands one to its call, beside the
/// guest's [`Instance`]. It implements [`AsContextMut`], so `&mut entered`
/// passes wherever wasmtime takes a store. Dropping it drops the store
/// first, then frees the slot.
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
