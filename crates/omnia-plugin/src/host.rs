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

use omnia_core::{Digest, HasExtensions, Host, Server};
use wasmtime::component::{Accessor, HasData, Linker};

use self::generated::Error;
use self::generated::omnia::plugins::loader;
use crate::error::LoadError;
use crate::loader::Plugins;
use crate::source::Location;

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

impl From<LoadError> for Error {
    fn from(error: LoadError) -> Self {
        match error {
            LoadError::Refused(detail) => Self::Refused(detail),
            LoadError::Unavailable(detail) => Self::Unavailable(detail),
            LoadError::Internal(detail) => Self::Internal(detail),
        }
    }
}

impl From<loader::Location> for Location {
    fn from(location: loader::Location) -> Self {
        match location {
            loader::Location::Declared(name) => Self::Declared(name),
            loader::Location::Path(path) => Self::Path(path),
            loader::Location::Registry(loader::RegistryRef { package, endpoint }) => {
                Self::Registry { package, endpoint }
            }
        }
    }
}

impl<T> loader::HostWithStore<T> for WasiPlugins {
    async fn load(
        accessor: &Accessor<T, Self>, from: loader::Location, digest: Option<String>,
    ) -> Result<loader::Plugin, Error> {
        let from = Location::from(from);
        let plugins = accessor
            .with(|mut store| store.get().plugins)
            .ok_or_else(|| LoadError::no_plugins(&from))?;
        let pin = digest
            .map(|digest| digest.parse::<Digest>())
            .transpose()
            .map_err(|error| LoadError::Refused(format!("{error:#}")))?;
        let plugin = plugins.load(from, pin).await?;
        Ok(loader::Plugin {
            id: plugin.id().to_string(),
            digest: plugin.digest().to_string(),
        })
    }
}

impl loader::Host for WasiPluginsCtxView {
    fn convert_error(&mut self, err: Error) -> wasmtime::Result<Error> {
        tracing::debug!("plugin load refused: {err}");
        Ok(err)
    }
}
