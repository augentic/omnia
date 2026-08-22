//! Traits implemented by WASI host crates (`wasi-*`).
//!
//! Each host crate provides a `WasiXxx` type implementing [`Host`] (and usually
//! [`Server`]) plus a default backend type implementing [`Backend`].

use std::fmt::Debug;
use std::future::Future;
use std::ops::Deref;
use std::sync::Arc;

use anyhow::Result;
use futures::future::BoxFuture;
use wasmtime::component::{Accessor, HasData, Linker, Resource, ResourceTable, ResourceTableError};

use crate::runtime::Runtime;

/// Result type for asynchronous host operations.
pub type FutureResult<T> = BoxFuture<'static, Result<T>>;

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
pub trait Server<B>: Debug + Sync + Send {
    /// Whether this host is a long-lived trigger server — one whose
    /// [`run`](Self::run) loops on a transport and returns only on shutdown
    /// (e.g. `WasiHttp`, `WasiMessaging`, `WasiWebSocket`).
    ///
    /// Defaults to `false`: a capability host with the no-op [`run`](Self::run)
    /// (e.g. `WasiKeyValue`, `WasiBlobstore`, `WasiOtel`). The `runtime!` macro
    /// reads this flag from the *type system* — to select which hosts to `run`.
    const IS_SERVER: bool = false;

    /// Start the service.
    ///
    /// This is typically implemented by services that instantiate (or run)
    /// wasm components.
    fn run(&self, _state: &Runtime<B>) -> impl Future<Output = Result<()>> {
        async { Ok(()) }
    }
}

/// Connect a host backend resource during runtime startup.
pub trait Backend: Sized + Sync + Send {
    /// The options used to connect to the backend.
    type ConnectOptions: FromEnv;

    /// Connect to the resource.
    #[must_use]
    fn connect() -> impl Future<Output = Result<Self>> {
        async { Self::connect_with(Self::ConnectOptions::load_env()?).await }
    }

    /// Connect with the specified options.
    fn connect_with(options: Self::ConnectOptions) -> impl Future<Output = Result<Self>>;
}

/// Create backend connection options from environment variables.
///
/// The method is `load_env` (not `from_env`) so it never shadows — or is shadowed
/// by — the builder-returning inherent `from_env` the `fromenv` derive emits
/// on option structs.
pub trait FromEnv: Sized {
    /// Load connection options from environment variables.
    ///
    /// # Errors
    ///
    /// Returns an error if required environment variables are missing or invalid.
    fn load_env() -> Result<Self>;
}

/// Connection options for a [`Backend`] that needs none.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoOptions;

impl FromEnv for NoOptions {
    fn load_env() -> Result<Self> {
        Ok(Self)
    }
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

/// Access to the store's resource table from a WASI ctx view.
///
/// Implemented by the `Wasi<Service>CtxView` types the [`wasi_view!`] macro
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
