//! Traits implemented by WASI host crates (`wasi-*`).
//!
//! Each host crate provides a `WasiXxx` type implementing [`Host`] plus a
//! default backend type implementing [`Backend`].

use std::fmt::Debug;
use std::future::Future;
use std::ops::Deref;
use std::sync::Arc;

use anyhow::Result;
use futures::future::BoxFuture;
use wasmtime::component::{Accessor, HasData, Linker, Resource, ResourceTable, ResourceTableError};

use crate::runtime::Runtime;
use crate::store::StoreCtx;

/// Result type for asynchronous host operations.
pub type FutureResult<T> = BoxFuture<'static, Result<T>>;

/// A deployment's backend bundle: what [`Runtime<B>`](Runtime) clones into
/// every store and every trigger server.
///
/// Implemented by every type with the shape, so a bound names the role
/// rather than its parts. The `runtime!` macro generates one bundle per
/// deployment; a hand-built bundle is any `Clone + Send + Sync + 'static`
/// type carrying the [`Provides`] impls its hosts need.
pub trait Backends: Clone + Send + Sync + 'static {}

impl<B: Clone + Send + Sync + 'static> Backends for B {}

/// A WASI host over the deployment's backend bundle `B`: linked into the
/// shared linker before instantiation, then run beside the guests.
///
/// The `runtime!` macro runs every linked host uniformly. A long-lived
/// trigger server (`WasiHttp`, `WasiMessaging`, `WasiWebSocket`) overrides
/// [`run`](Self::run) to loop on its transport until shutdown; a capability
/// host (`WasiKeyValue`, `WasiBlobstore`, `WasiOtel`) keeps the no-op
/// default, which resolves at once.
pub trait Host<B: Backends>: Send + Sync {
    /// Link the host's generated bindings into the deployment linker.
    ///
    /// # Errors
    ///
    /// Returns the linking error from the service's generated bindings.
    fn add_to_linker(linker: &mut Linker<StoreCtx<B>>) -> Result<()>;

    /// Run the host until shutdown.
    fn run(_runtime: &Runtime<B>) -> impl Future<Output = Result<()>> + Send {
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
///
/// A host over one client trait derives the impl with
/// [`#[derive(HostCtx)]`](derive@crate::HostCtx), naming the trait in
/// `#[host_ctx(client = ..)]`; its view is [`HostView`] over that trait object.
/// Given a bundle `B: Provides<Self>`, [`StoreCtx::view`] assembles the view
/// for every store. A host with another borrow shape, such as `WasiHttp`,
/// implements it by hand.
///
/// # Examples
///
/// ```
/// use omnia_core::{HostCtx, HostView, Provides, StoreCtx};
///
/// pub trait FooClient {}
///
/// /// Host-side service for `omnia:foo`.
/// #[derive(Debug, HostCtx)]
/// #[host_ctx(client = FooClient)]
/// pub struct WasiFoo;
///
/// #[derive(Clone)]
/// struct Backends<C: Clone>(C);
///
/// impl<C: FooClient + Clone + Send + Sync + 'static> Provides<WasiFoo> for Backends<C> {
///     fn borrow(&mut self) -> &dyn FooClient {
///         &self.0
///     }
/// }
///
/// fn view<C: FooClient + Clone + Send + Sync + 'static>(
///     store: &mut StoreCtx<Backends<C>>,
/// ) -> HostView<'_, dyn FooClient> {
///     store.view::<WasiFoo>()
/// }
/// ```
pub trait HostCtx: HasData {
    /// What this host borrows from the backend bundle.
    type Borrow<'a>;

    /// Assemble the linker-facing view from the borrow and the store's
    /// resource table.
    fn view<'a>(borrow: Self::Borrow<'a>, table: &'a mut ResourceTable) -> Self::Data<'a>;
}

/// The one bundle-accessor trait: a [`Backends`] bundle that yields host `H`'s
/// borrow.
///
/// `runtime!` deployments generate one impl per `hosts:` row; hand-built
/// bundles implement it directly.
pub trait Provides<H: HostCtx>: Backends {
    /// Borrow what host `H` needs from the bundle.
    fn borrow(&mut self) -> H::Borrow<'_>;
}

/// The linker-facing view of a host over one client: its backend and the
/// store's resource table, borrowed for one host call.
///
/// A host that names its client in [`#[derive(HostCtx)]`](derive@crate::HostCtx)
/// has `HostView<'a, dyn XClient>` as its [`HasData::Data`], and implements
/// its bindgen `Host*` traits on it: a host function reaches the backend
/// through [`client`](Self::client) and WIT resources through
/// [`table`](Self::table). [`StoreCtx::view`] assembles one per host call.
// `C: 'a` is not redundant: it makes `HostView<'a, dyn XClient>` default the
// object lifetime to `'a`, the same as the `&'a dyn XClient` it is built
// from, for a client trait with or without a `'static` bound.
pub struct HostView<'a, C: ?Sized + 'a> {
    /// The host's backend, borrowed from the deployment's bundle.
    pub client: &'a C,
    /// The store's resource table.
    pub table: &'a mut ResourceTable,
}

impl<C: ?Sized> HasTable for HostView<'_, C> {
    fn table(&mut self) -> &mut ResourceTable {
        self.table
    }
}

/// Connect a host backend resource during runtime startup.
pub trait Backend: Sized + Sync + Send {
    /// The options used to connect to the backend.
    type ConnectOptions: FromEnv;

    /// Connect to the resource.
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

/// Access to the store's resource table from a WASI view.
///
/// [`HostView`] implements it for every client, enabling the generic
/// [`get_cloned`] table lookup.
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
