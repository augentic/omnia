use omnia_core::Digest;
use wasmtime::component::Accessor;

use crate::error::LoadError;
use crate::host::generated::Error;
use crate::host::generated::omnia::plugins::{loader, types};
use crate::host::{WasiPlugins, WasiPluginsView};
use crate::loader::{LoadConflict, Plugin};
use crate::source::Location;

impl<T> loader::HostWithStore<T> for WasiPlugins {
    async fn load(
        accessor: &Accessor<T, Self>, source: types::Source, when: Option<types::Digest>,
    ) -> Result<Result<types::Plugin, types::Conflict>, Error> {
        let from = Location::from(source);
        let plugins = accessor
            .with(|mut store| store.get().plugins)
            .ok_or_else(|| LoadError::no_plugins(&from))?;
        let pin = when
            .map(|digest| digest.parse::<Digest>())
            .transpose()
            .map_err(|error| LoadError::Invalid(format!("{error:#}")))?;
        let loaded = plugins.load(from, pin).await?;
        Ok(loaded.map(|plugin| types::Plugin::from(&plugin)).map_err(types::Conflict::from))
    }
}

impl loader::Host for WasiPluginsView {}

impl From<LoadError> for Error {
    fn from(error: LoadError) -> Self {
        match error {
            LoadError::NotFound(detail) => Self::NotFound(detail),
            LoadError::Denied(detail) => Self::Denied(detail),
            LoadError::Invalid(detail) => Self::Invalid(detail),
            LoadError::Unsupported(detail) => Self::Unsupported(detail),
            LoadError::Unavailable(detail) => Self::Unavailable(detail),
            LoadError::Other(detail) => Self::Other(detail),
        }
    }
}

impl From<types::Source> for Location {
    fn from(source: types::Source) -> Self {
        match source {
            types::Source::Declared(name) => Self::Declared(name),
            types::Source::Path(path) => Self::Path(path),
            types::Source::Release(types::Release { reference, registry }) => Self::Registry {
                package: reference,
                endpoint: registry,
            },
        }
    }
}

impl From<&Plugin> for types::Plugin {
    fn from(plugin: &Plugin) -> Self {
        Self {
            name: plugin.id().to_string(),
            digest: plugin.digest().to_string(),
            exports: plugin.exports().to_vec(),
        }
    }
}

impl From<LoadConflict> for types::Conflict {
    fn from(conflict: LoadConflict) -> Self {
        Self {
            found: conflict.found().to_string(),
            current: conflict.current().map(types::Plugin::from),
        }
    }
}
