//! Parsers for environment variables, for a [`FromEnv`](derive@crate::FromEnv)
//! field's `with` key, and the patterns that name variables.
//!
//! A `with` parser is any `fn(&str) -> Result<T, E>` whose `E` implements
//! [`Display`](std::fmt::Display). These are the ones several option structs
//! share.
//!
//! A pattern is a whole variable name, or a prefix of one that ends in `*`.
//! [`pattern`] checks one, and [`matches`] applies it to a name.

#[doc(hidden)]
pub mod derive;

use std::convert::Infallible;
use std::fmt;
use std::num::ParseIntError;
use std::path::PathBuf;
use std::time::Duration;

pub use crate::names::matches;

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

/// Checks a variable pattern: a whole variable name, or a prefix of one that ends in `*`.
///
/// A name is `[A-Za-z_][A-Za-z0-9_]*`, and `*` alone names every variable.
///
/// # Errors
///
/// Returns a [`PatternError`] naming an entry of any other shape.
pub fn pattern(value: &str) -> Result<String, PatternError> {
    let mut bytes = value.strip_suffix('*').unwrap_or(value).bytes();
    let valid = value == "*"
        || (bytes.next().is_some_and(|first| first == b'_' || first.is_ascii_alphabetic())
            && bytes.all(|byte| byte == b'_' || byte.is_ascii_alphanumeric()));
    if valid {
        Ok(value.to_owned())
    } else {
        Err(PatternError {
            entry: value.to_owned(),
        })
    }
}

/// A variable pattern that is neither a variable name nor a prefix of one ending in `*`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PatternError {
    entry: String,
}

impl fmt::Display for PatternError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "`{}` is not a variable name, or a prefix of one that ends in `*`", self.entry)
    }
}

impl std::error::Error for PatternError {}

// Unit tests by design: the check is a pure function every list of patterns
// shares.
#[cfg(test)]
mod tests {
    use super::pattern;

    #[test]
    fn shapes() {
        for entry in ["*", "HOME", "ORDERS_*", "_PRIVATE"] {
            assert_eq!(pattern(entry).as_deref(), Ok(entry));
        }
        for entry in ["", "**", "9LIVES", "ORDERS-URL", "A*B", "HOME "] {
            let error = pattern(entry).expect_err("not a variable pattern");
            assert!(error.to_string().starts_with(&format!("`{entry}`")), "{error}");
        }
    }
}
