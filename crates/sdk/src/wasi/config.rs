//! [`Config`] over `wasi:config`.

use std::future::Future;

use omnia_wasi_config::store;

use crate::{CapabilityError, Config, ErrorKind, WasiProvider};

impl Config for WasiProvider {
    fn get(
        &self, key: &str,
    ) -> impl Future<Output = Result<Option<String>, CapabilityError>> + Send {
        std::future::ready(store::get(key).map_err(CapabilityError::from))
    }
}

impl From<store::Error> for CapabilityError {
    fn from(err: store::Error) -> Self {
        match err {
            store::Error::Upstream(message) => Self::new(ErrorKind::Unavailable, message),
            store::Error::Io(message) => Self::new(ErrorKind::Other, message),
        }
    }
}
