//! `omnia:config`, the configuration a deployment sets for its guest.
//!
//! [`get`] reads a key as text. [`require`] and [`parse`] read it as a typed
//! value, and a [`Settings`](crate::Settings) struct loads a part's keys
//! together.

use std::fmt;
use std::str::FromStr;

pub use crate::bindings::imports::omnia::config::*;
use crate::{Error, Result};

/// The value set for `key`, or `None` when it is unset.
///
/// # Errors
///
/// The host's failure, under its code, when the configuration source does
/// not answer.
pub async fn get(key: &str) -> Result<Option<String>> {
    Ok(store::get(key.to_owned()).await?)
}

/// The value set for `key`, parsed as `T`.
///
/// # Errors
///
/// - [`ServerError`](crate::ErrorKind::ServerError) with the code `not-found` when `key`
///   is unset, or `invalid` when its value does not parse as `T`.
/// - The host's failure, under its code, when the configuration source does
///   not answer.
pub async fn require<T>(key: &str) -> Result<T>
where
    T: FromStr,
    T::Err: fmt::Display,
{
    required(key, parse(key).await?)
}

/// The value set for `key` parsed as `T`, or `None` when it is unset.
///
/// # Errors
///
/// - [`ServerError`](crate::ErrorKind::ServerError) with the code `invalid` when the
///   value does not parse as `T`.
/// - The host's failure, under its code, when the configuration source does
///   not answer.
pub async fn parse<T>(key: &str) -> Result<Option<T>>
where
    T: FromStr,
    T::Err: fmt::Display,
{
    typed(key, get(key).await?)
}

fn required<T>(key: &str, value: Option<T>) -> Result<T> {
    value.ok_or_else(|| Error::server_error("not-found", format!("`{key}` is not set")))
}

fn typed<T>(key: &str, value: Option<String>) -> Result<Option<T>>
where
    T: FromStr,
    T::Err: fmt::Display,
{
    value
        .map(|value| {
            value
                .parse::<T>()
                .map_err(|err| Error::server_error("invalid", format!("`{key}` is invalid: {err}")))
        })
        .transpose()
}
