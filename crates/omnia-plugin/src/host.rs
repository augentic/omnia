//! Host surface for `omnia:plugins/loader`.

mod generated {
    pub use self::omnia::plugins::loader::Error;
    pub use crate::Plugin;

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

use omnia_core::{HasExtensions as _, Host, Server, StoreCtx};
use wasmtime::component::{Access, Accessor, HasData, Linker, Resource, ResourceTable};

pub use self::generated::Error;
use self::generated::omnia::plugins::loader;
use crate::Location;
use crate::loader::{Plugin, PluginLoader as _, Plugins};

/// Host-side service for `omnia:plugins` — the loader capability this crate
/// implements over the runtime's admission seam.
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
    /// The runtime's plugin-load capability; `None` when the deployment
    /// installed no [`Plugins`] extension, where every load refuses.
    pub plugins: Option<Arc<Plugins>>,
    /// Mutable reference to the table used to manage resources.
    pub table: &'a mut ResourceTable,
}

impl<B: Send + 'static> WasiPluginsView for StoreCtx<B> {
    fn plugins(&mut self) -> WasiPluginsCtxView<'_> {
        WasiPluginsCtxView {
            plugins: self.extensions().get::<Plugins>(),
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

impl<T> loader::HostWithStore<T> for WasiPlugins {
    async fn load(
        accessor: &Accessor<T, Self>, package: String, from: loader::Location,
        digest: Option<String>,
    ) -> Result<Resource<Plugin>, Error> {
        let plugins = accessor
            .with(|mut store| store.get().plugins)
            .ok_or_else(|| Error::Internal(
                format!("this deployment has no plugins; compile one in (`plugins: {{ locations: [...] }}`) to load `{package}`")
            ))?;
        let plugin = plugins.load(package, from.into(), digest).await?;
        Ok(accessor.with(|mut store| store.get().table.push(plugin))?)
    }
}

impl<T> loader::HostPluginWithStore<T> for WasiPlugins {
    // `host` and `accessor` follow the generated trait's parameter names.
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

omnia_core::host_error!(Error, Internal);
