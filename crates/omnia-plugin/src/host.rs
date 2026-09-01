//! Host surface for `omnia:plugins/loader`.

mod generated {
    pub use self::omnia::plugins::loader::Error;

    wasmtime::component::bindgen!({
        world: "imports",
        path: "wit",
        imports: {
            default: store | tracing | trappable,
        },
        trappable_error_type: {
            "omnia:plugins/loader.error" => Error,
        },
    });
}

use std::sync::Arc;

use omnia_core::{HasExtensions, Host, Server};
use wasmtime::component::{Accessor, HasData, Linker};

pub use self::generated::Error;
use self::generated::omnia::plugins::loader;
use crate::Location;
use crate::loader::{Plugins, missing_plugins};

/// Host-side service for `omnia:plugins` — the loader capability this crate
/// implements over the runtime's admission seam.
#[derive(Debug)]
pub struct WasiPlugins;

impl HasData for WasiPlugins {
    type Data<'a> = WasiPluginsCtxView;
}

impl<T> Host<T> for WasiPlugins
where
    T: HasExtensions + 'static,
{
    fn add_to_linker(linker: &mut Linker<T>) -> anyhow::Result<()> {
        Ok(loader::add_to_linker::<_, Self>(linker, |store| WasiPluginsCtxView {
            plugins: store.extensions().get::<Plugins>(),
        })?)
    }
}

impl<B> Server<B> for WasiPlugins {}

/// View over the store's plugin-load capability.
pub struct WasiPluginsCtxView {
    /// The runtime's plugin-load capability; `None` when the deployment
    /// installed no [`Plugins`] extension, where every load refuses.
    pub plugins: Option<Arc<Plugins>>,
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
    ) -> Result<loader::Plugin, Error> {
        let plugins = accessor
            .with(|mut store| store.get().plugins)
            .ok_or_else(|| missing_plugins(&package))?;
        let plugin = plugins.load(&package, from.into(), digest.as_deref()).await?;
        Ok(loader::Plugin {
            id: plugin.id().to_string(),
            digest: plugin.digest().to_owned(),
        })
    }
}

impl loader::Host for WasiPluginsCtxView {
    fn convert_error(&mut self, err: Error) -> wasmtime::Result<Error> {
        tracing::debug!("plugin load refused: {err}");
        Ok(err)
    }
}
