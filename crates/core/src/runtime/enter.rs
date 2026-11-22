//! The one way a trigger or the command driver runs a guest:
//! [`Runtime::enter`].

use anyhow::Result;
use wasmtime::Store;
use wasmtime::component::Instance;

use super::Runtime;
use crate::chain::ChainCtx;
use crate::host::Backends;
use crate::registry::Guest;
use crate::store::StoreCtx;

impl<B: Backends> Runtime<B> {
    /// Instantiate `guest` on a fresh store at `chain`: how a trigger or the
    /// command driver runs a guest.
    ///
    /// The store is built for `guest` alone and drops with the invocation
    /// (instance-per-call).
    ///
    /// # Errors
    ///
    /// Returns an error if the component cannot be instantiated.
    ///
    /// # Panics
    ///
    /// Panics if `MAX_FUEL` is set but the engine was built without fuel
    /// metering.
    pub async fn enter(
        &self, guest: &Guest<B>, chain: ChainCtx,
    ) -> Result<(Store<StoreCtx<B>>, Instance)> {
        let mut store = self.build_store(self.store_ctx(guest, chain));
        let instance = guest.instance_pre().instantiate_async(&mut store).await?;
        tracing::debug!(guest = %guest.id(), "component instantiated");
        Ok((store, instance))
    }
}
