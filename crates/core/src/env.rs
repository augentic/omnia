//! Parsers for environment variables, for a [`FromEnv`](derive@crate::FromEnv)
//! field's `with` key.
//!
//! A `with` parser is any `fn(&str) -> Result<T, E>` whose `E` implements
//! [`Display`](std::fmt::Display). These are the ones several option structs
//! share.

#[doc(hidden)]
pub mod derive;

use std::convert::Infallible;
use std::num::ParseIntError;
use std::path::PathBuf;
use std::time::Duration;

/// Splits a comma-separated list, trimming each entry and dropping the empty ones.
///
/// # Errors
///
/// Never fails. The [`Infallible`] error is the shape a `with` parser returns.
#[must_use]
pub fn list(value: &str) -> Result<Vec<String>, Infallible> {
    Ok(value
        .split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .map(str::to_owned)
        .collect())
}

/// Parses a whole number of milliseconds as a [`Duration`].
///
/// # Errors
///
/// Returns the [`ParseIntError`] of a value that is not an unsigned integer.
pub fn millis(value: &str) -> Result<Duration, ParseIntError> {
    value.parse().map(Duration::from_millis)
}

/// Splits a list of paths in the platform's `PATH` syntax, dropping the empty entries.
///
/// The separator is the one [`std::env::split_paths`] uses: `:` on Unix and
/// `;` on Windows.
///
/// # Errors
///
/// Never fails. The [`Infallible`] error is the shape a `with` parser returns.
#[must_use]
pub fn paths(value: &str) -> Result<Vec<PathBuf>, Infallible> {
    Ok(std::env::split_paths(value).filter(|path| !path.as_os_str().is_empty()).collect())
}
