//! Traits implemented by WASI host crates (`wasi-*`).
//!
//! Each host crate provides a `WasiXxx` type implementing [`Host`] (and usually
//! [`Server`]) plus a default backend type implementing [`Connect`](crate::Connect).

use std::fmt::Debug;
use std::future::Future;
use std::ops::Deref;
use std::sync::Arc;

use anyhow::Result;
use wasmtime::component::{Accessor, HasData, Linker, Resource, ResourceTable, ResourceTableError};

use crate::runtime::Runtime;

/// Link a WASI host's generated bindings into the deployment linker.
pub trait Host<T>: Debug + Sync + Send {
    /// Link the host's dependencies prior to component instantiation.
    ///
    /// # Errors
    ///
    /// Returns an linking error(s) from the service's generated bindings.
    fn add_to_linker(linker: &mut Linker<T>) -> Result<()>;
}

/// Start a WASI host — typically a long-lived trigger server.
///
/// Parameterized by the deployment's backend bundle `B` so [`run`](Self::run)
/// receives the concrete [`Runtime<B>`].
///
/// The `runtime!` macro runs every linked host uniformly: a long-lived
/// trigger server (e.g. `WasiHttp`, `WasiMessaging`, `WasiWebSocket`)
/// overrides [`run`](Self::run) to loop on its transport until shutdown,
/// while a capability host (e.g. `WasiKeyValue`, `WasiBlobStore`,
/// `WasiOtel`) keeps the no-op default, which resolves immediately.
pub trait Server<B>: Debug + Sync + Send {
    /// Start the service.
    ///
    /// This is typically implemented by services that instantiate (or run)
    /// wasm components.
    fn run(&self, _state: &Runtime<B>) -> impl Future<Output = Result<()>> {
        async { Ok(()) }
    }
}

/// One host's linker-facing data assembly, carried in the type system.
///
/// [`Borrow`](Self::Borrow) is what the host borrows from the deployment's
/// backend bundle; [`view`](Self::view) assembles the linker-facing
/// [`HasData::Data`] view from that borrow plus the store's resource table.
/// The `runtime!` macro never derives names from the host type — everything
/// it needs rides these associated items.
pub trait HostCtx: HasData {
    /// What this host borrows from the backend bundle.
    type Borrow<'a>;

    /// Assemble the linker-facing view from the borrow and the store's
    /// resource table.
    fn view<'a>(borrow: Self::Borrow<'a>, table: &'a mut ResourceTable) -> Self::Data<'a>;
}

/// The one bundle-accessor trait: a backend bundle that yields host `H`'s
/// borrow.
///
/// `runtime!` deployments generate one impl per `hosts:` row; hand-built
/// bundles implement it directly.
pub trait Provides<H: HostCtx>: Send {
    /// Borrow what host `H` needs from the bundle.
    fn borrow(&mut self) -> H::Borrow<'_>;
}

/// Resource-table proxy over a shared backend handle.
///
/// WASI host crates alias this per resource (e.g.
/// `type BucketProxy = Proxy<dyn Bucket>`) to bridge WIT resources to their
/// backend trait objects.
pub struct Proxy<T: ?Sized>(pub Arc<T>);

impl<T: ?Sized> Clone for Proxy<T> {
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
}

impl<T: ?Sized + Debug> Debug for Proxy<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Proxy").field(&self.0).finish()
    }
}

impl<T: ?Sized> Deref for Proxy<T> {
    type Target = Arc<T>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// Access to the store's resource table from a WASI view.
///
/// Implemented by the `Wasi<Service>View` types the [`wasi_view!`] macro
/// generates, enabling the generic [`get_cloned`] table lookup.
///
/// [`wasi_view!`]: crate::wasi_view
pub trait HasTable {
    /// The store's resource table.
    fn table(&mut self) -> &mut ResourceTable;
}

/// Clone `resource`'s host-side entry out of the store's resource table.
///
/// # Errors
///
/// Returns the table error when the handle is stale or belongs to another
/// store.
pub fn get_cloned<T, D, R>(
    accessor: &Accessor<T, D>, resource: &Resource<R>,
) -> Result<R, ResourceTableError>
where
    D: HasData,
    for<'a> D::Data<'a>: HasTable,
    R: Clone + Send + 'static,
{
    accessor.with(|mut store| store.get().table().get(resource).cloned())
}
