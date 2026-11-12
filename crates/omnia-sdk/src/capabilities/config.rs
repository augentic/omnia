//! Configuration lookup capability.

use std::fmt;
use std::future::Future;
use std::str::FromStr;

use crate::capability::{Error, ErrorKind, Result};

/// Read the configuration a deployment sets for its guest.
///
/// An unset key is `Ok(None)`, not an error. Typed reads over the string
/// values are [`ConfigExt`].
#[omnia_guest_macros::capability]
pub trait Config: Send + Sync {
    /// The value set for `key`, or `None` when it is unset.
    fn get(&self, key: &str) -> impl Future<Output = Result<Option<String>>> + Send;
}

/// Typed reads over any [`Config`].
///
/// Blanket-implemented for every [`Config`]; never implemented by hand.
pub trait ConfigExt: Config {
    /// The value set for `key`, parsed as `T`.
    ///
    /// An unset key fails with [`ErrorKind::NotFound`], and a value that does
    /// not parse as `T` with [`ErrorKind::Invalid`].
    fn require<T>(&self, key: &str) -> impl Future<Output = Result<T>> + Send
    where
        T: FromStr,
        T::Err: fmt::Display,
    {
        async move {
            self.parse(key)
                .await?
                .ok_or_else(|| Error::new(ErrorKind::NotFound, format!("`{key}` is not set")))
        }
    }

    /// The value set for `key` parsed as `T`, or `None` when it is unset.
    ///
    /// A value that does not parse as `T` fails with [`ErrorKind::Invalid`].
    fn parse<T>(&self, key: &str) -> impl Future<Output = Result<Option<T>>> + Send
    where
        T: FromStr,
        T::Err: fmt::Display,
    {
        async move {
            let Some(value) = self.get(key).await? else {
                return Ok(None);
            };
            value
                .parse::<T>()
                .map(Some)
                .map_err(|err| Error::new(ErrorKind::Invalid, format!("`{key}` is invalid: {err}")))
        }
    }
}

impl<P: Config + ?Sized> ConfigExt for P {}

#[cfg(all(target_arch = "wasm32", target_os = "wasi"))]
impl Config for crate::WasiProvider {
    fn get(&self, key: &str) -> impl Future<Output = Result<Option<String>>> + Send {
        std::future::ready(omnia_wasi_config::store::get(key).map_err(Error::from))
    }
}

#[cfg(all(target_arch = "wasm32", target_os = "wasi"))]
impl From<omnia_wasi_config::store::Error> for Error {
    fn from(err: omnia_wasi_config::store::Error) -> Self {
        use omnia_wasi_config::store::Error as Wire;

        match err {
            Wire::Upstream(message) => Self::new(ErrorKind::Unavailable, message),
            Wire::Io(message) => Self::new(ErrorKind::Other, message),
        }
    }
}
