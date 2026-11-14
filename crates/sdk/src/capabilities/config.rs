//! Configuration lookup capability.

use std::fmt;
use std::future::Future;
use std::str::FromStr;

use crate::{Error, Result};

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
    /// An unset key fails with the code `not-found`, and a value that does
    /// not parse as `T` with the code `invalid`.
    fn require<T>(&self, key: &str) -> impl Future<Output = Result<T>> + Send
    where
        T: FromStr,
        T::Err: fmt::Display,
    {
        async move {
            self.parse(key)
                .await?
                .ok_or_else(|| Error::server_error("not-found", format!("`{key}` is not set")))
        }
    }

    /// The value set for `key` parsed as `T`, or `None` when it is unset.
    ///
    /// A value that does not parse as `T` fails with the code `invalid`.
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
                .map_err(|err| Error::server_error("invalid", format!("`{key}` is invalid: {err}")))
        }
    }
}

impl<P: Config + ?Sized> ConfigExt for P {}
