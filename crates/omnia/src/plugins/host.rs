//! Host surface for `omnia:plugins/loader`.

mod generated {
    pub use self::omnia::plugins::loader::Error;
    pub use crate::plugins::Plugin;

    wasmtime::component::bindgen!({
        world: "imports",
        path: "wit",
        imports: {
            default: store | tracing | trappable,
        },
        with: {
            "omnia:plugins/loader.plugin": Plugin,
        },
        trappable_error_type: {
            "omnia:plugins/loader.error" => Error,
        },
    });
}

use std::sync::Arc;

use wasmtime::component::{Access, Accessor, HasData, Linker, Resource, ResourceTable};

use self::generated::Error;
use self::generated::omnia::plugins::loader;
use crate::plugins::{LoadError, Location, Plugin, PluginLoader};
use crate::{Host, Server, StoreCtx};

/// Host-side service for `omnia:plugins` — the loader capability the runtime
/// core implements itself.
#[derive(Debug)]
pub struct WasiPlugins;

impl HasData for WasiPlugins {
    type Data<'a> = WasiPluginsCtxView<'a>;
}

impl<T> Host<T> for WasiPlugins
where
    T: WasiPluginsView + 'static,
{
    fn add_to_linker(linker: &mut Linker<T>) -> anyhow::Result<()> {
        Ok(loader::add_to_linker::<_, Self>(linker, T::plugins)?)
    }
}

impl<B> Server<B> for WasiPlugins {}

/// Provides internal WASI Plugins state.
///
/// Implemented by the `T` in `Linker<T>`: a single type shared across every
/// WASI component in a runtime build.
pub trait WasiPluginsView: Send {
    /// Borrow a `WasiPluginsCtxView` from a mutable reference to self.
    fn plugins(&mut self) -> WasiPluginsCtxView<'_>;
}

/// Borrowed view over the store's plugin-load capability and resource table.
pub struct WasiPluginsCtxView<'a> {
    /// The runtime's plugin-load capability; `None` in hand-built store
    /// contexts, where every load refuses.
    pub loader: Option<&'a Arc<dyn PluginLoader>>,
    /// Mutable reference to the table used to manage resources.
    pub table: &'a mut ResourceTable,
}

impl<B: Send + 'static> WasiPluginsView for StoreCtx<B> {
    fn plugins(&mut self) -> WasiPluginsCtxView<'_> {
        WasiPluginsCtxView {
            loader: self.base.loader.as_ref(),
            table: &mut self.base.table,
        }
    }
}

impl From<loader::Location> for Location {
    fn from(location: loader::Location) -> Self {
        match location {
            loader::Location::Registry(registry) => Self::Registry(registry),
            loader::Location::Path(path) => Self::Path(path),
        }
    }
}

impl From<LoadError> for Error {
    fn from(error: LoadError) -> Self {
        match error {
            LoadError::UnsupportedLocation(reason) => Self::LocationUnsupported(reason),
            LoadError::AcquireFailed(reason) => Self::AcquireFailed(reason),
            LoadError::InvalidDigest(reason) => Self::InvalidDigest(reason),
            LoadError::DigestMismatch(reason) => Self::DigestMismatch(reason),
            LoadError::ArtifactRefused(reason) => Self::ArtifactRefused(reason),
            LoadError::SeamMissing(reason) => Self::SeamMissing(reason),
            LoadError::AlreadyActive(reason) => Self::AlreadyActive(reason),
            LoadError::Internal(reason) => Self::Internal(reason),
        }
    }
}

impl<T> loader::HostWithStore<T> for WasiPlugins {
    async fn load(
        accessor: &Accessor<T, Self>, package: String, from: loader::Location,
        digest: Option<String>,
    ) -> Result<Resource<Plugin>, Error> {
        let loader = accessor
            .with(|mut store| store.get().loader.map(Arc::clone))
            .ok_or_else(|| Error::Internal("this store carries no plugin loader".to_owned()))?;
        let plugin = loader.load(package, from.into(), digest).await?;
        Ok(accessor.with(|mut store| store.get().table.push(plugin))?)
    }
}

impl<T> loader::HostPluginWithStore<T> for WasiPlugins {
    fn id(mut host: Access<'_, T, Self>, self_: Resource<Plugin>) -> wasmtime::Result<String> {
        Ok(host.get().table.get(&self_)?.id().to_string())
    }

    fn digest(mut host: Access<'_, T, Self>, self_: Resource<Plugin>) -> wasmtime::Result<String> {
        Ok(host.get().table.get(&self_)?.digest().to_owned())
    }

    fn drop(mut accessor: Access<'_, T, Self>, rep: Resource<Plugin>) -> wasmtime::Result<()> {
        Ok(accessor.get().table.delete(rep).map(|_| ())?)
    }
}

impl loader::Host for WasiPluginsCtxView<'_> {
    fn convert_error(&mut self, err: Error) -> wasmtime::Result<Error> {
        tracing::debug!("plugin load refused: {err}");
        Ok(err)
    }
}

impl loader::HostPlugin for WasiPluginsCtxView<'_> {}

crate::host_error!(Error, Internal);
