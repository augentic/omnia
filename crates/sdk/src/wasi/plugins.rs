//! [`Plugins`] over `omnia:plugins/loader`.

use omnia_plugin::loader;

use crate::plugins::{Digest, Error, Location, Plugin};
use crate::{Plugins, WasiProvider};

impl Plugins for WasiProvider {
    async fn load(&self, from: &Location, digest: Option<&Digest>) -> Result<Plugin, Error> {
        let from = loader::Location::from(from);
        let digest = digest.map(ToString::to_string);
        let loaded = loader::load(from, digest).await?;
        let digest = loaded.digest.parse().map_err(|error: Error| {
            Error::Internal(format!("host reported a malformed digest: {error}"))
        })?;
        Ok(Plugin::new(loaded.id, digest, loaded.exports))
    }
}

impl From<&Location> for loader::Location {
    fn from(location: &Location) -> Self {
        match location {
            Location::Declared(name) => Self::Declared(name.clone()),
            Location::Path(path) => Self::Path(path.clone()),
            Location::Registry { package, endpoint } => Self::Registry(loader::RegistryRef {
                package: package.clone(),
                endpoint: endpoint.clone(),
            }),
        }
    }
}

impl From<loader::Error> for Error {
    fn from(error: loader::Error) -> Self {
        match error {
            loader::Error::Refused(detail) => Self::Refused(detail),
            loader::Error::Unavailable(detail) => Self::Unavailable(detail),
            loader::Error::Internal(detail) => Self::Internal(detail),
        }
    }
}
