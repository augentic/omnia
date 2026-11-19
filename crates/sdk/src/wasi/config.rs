//! [`Config`] over `omnia:config`.

use omnia_wasi_config::omnia::{store, types};

use crate::{Config, Error, Result, WasiProvider};

impl Config for WasiProvider {
    async fn get(&self, key: &str) -> Result<Option<String>> {
        Ok(store::get(key.to_owned()).await?)
    }
}

impl From<types::Error> for Error {
    fn from(err: types::Error) -> Self {
        match err {
            types::Error::NotFound(message) => Self::server_error("not-found", message),
            types::Error::Denied(message) => Self::server_error("denied", message),
            types::Error::Invalid(message) => Self::server_error("invalid", message),
            types::Error::Unsupported(message) => Self::server_error("unsupported", message),
            types::Error::Unavailable(message) => Self::bad_gateway("unavailable", message),
            types::Error::Other(message) => Self::server_error("other", message),
        }
    }
}
