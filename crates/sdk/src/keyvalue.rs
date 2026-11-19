//! `omnia:keyvalue`, the buckets of key-value state a deployment declares.
//!
//! A guest opens a [`Bucket`] with [`store::open`], [`DEFAULT`] unless the
//! deployment declares another. Beside the generated methods, a bucket reads
//! and writes JSON and swaps bytes it expects to find. A TTL becomes a
//! [`Lifetime`] through `From`.
//!
//! The deployment declares its buckets, and opening an undeclared one fails
//! with the code `not-found`. Every backend accepts a portable key, and a
//! backend may reject any other with the code `invalid`. A portable key:
//!
//! - is one or more segments joined by `.`;
//! - has no empty segment, and holds only ASCII letters, digits, `_`, and
//!   `-` in each;
//! - is at most 255 bytes long.
//!
//! A TTL is a lifetime the store enforces, and a zero TTL fails with the
//! code `invalid` and writes nothing.

use std::time::Duration;

use serde::Serialize;
use serde::de::DeserializeOwned;

pub use crate::bindings::imports::omnia::keyvalue::{bucket, keys, store, types};
use crate::keyvalue::bucket::Bucket;
use crate::keyvalue::types::Lifetime;
use crate::{Error, Result};

impl Bucket {
    /// The JSON value at `key` decoded as `T`, or `None` when it is absent.
    ///
    /// # Errors
    ///
    /// - [`ServerError`](Error::ServerError) with the code `invalid` when the
    ///   stored bytes do not decode as `T`.
    /// - The host's failure, under its code.
    pub async fn get_json<T: DeserializeOwned>(&self, key: &str) -> Result<Option<T>> {
        let entry = self.get(key.to_owned()).await?;
        entry.map(|entry| decode(key, &entry.value)).transpose()
    }

    /// Store `value` at `key` as JSON, replacing any entry and its lifetime.
    ///
    /// A `ttl` of `None` keeps the value until it is deleted.
    ///
    /// # Errors
    ///
    /// - [`ServerError`](Error::ServerError) with the code `invalid` when
    ///   `value` does not serialize, or `ttl` is zero.
    /// - The host's failure, under its code.
    pub async fn set_json<T: Serialize + Sync + ?Sized>(
        &self, key: &str, value: &T, ttl: Option<Duration>,
    ) -> Result<()> {
        let bytes = encode(key, value)?;
        self.set(key.to_owned(), bytes, ttl.into(), None).await??;
        Ok(())
    }

    /// Replace the value at `key` with `new` while it still equals `current`.
    ///
    /// A `current` of `None` means the key is absent, so the swap creates the
    /// entry with `ttl`. An existing entry keeps the time it has left. A
    /// mismatch changes nothing and returns the entry it found. There is no
    /// retry loop.
    ///
    /// # Errors
    ///
    /// - [`ServerError`](Error::ServerError) with the code `invalid` when
    ///   `ttl` is zero, whether or not the value matches.
    /// - The host's failure, under its code.
    pub async fn compare_and_swap(
        &self, key: &str, current: Option<&[u8]>, new: &[u8], ttl: Option<Duration>,
    ) -> Result<Result<(), types::Conflict>> {
        if ttl == Some(Duration::ZERO) {
            return Err(zero_ttl());
        }

        // write under the revision the expected bytes are stored at
        let when = match current {
            None => types::Precondition::Absent,
            Some(expected) => match self.get(key.to_owned()).await? {
                Some(entry) if entry.value == expected => {
                    types::Precondition::Matches(entry.revision)
                }
                found => return Ok(Err(types::Conflict { current: found })),
            },
        };
        let lifetime = ttl.map_or(Lifetime::Keep, |ttl| Lifetime::KeepOrTtlMs(ttl_ms(ttl)));
        let swapped = self.set(key.to_owned(), new.to_vec(), lifetime, Some(when)).await?;
        Ok(swapped.map(|_revision| ()))
    }
}

/// The milliseconds `ttl` spans, rounded up.
///
/// A sub-millisecond TTL still expires, and only [`Duration::ZERO`] is zero,
/// which a store refuses.
#[must_use]
pub fn ttl_ms(ttl: Duration) -> u64 {
    u64::try_from(ttl.as_nanos().div_ceil(1_000_000)).unwrap_or(u64::MAX)
}

impl From<Duration> for Lifetime {
    fn from(ttl: Duration) -> Self {
        Self::TtlMs(ttl_ms(ttl))
    }
}

impl From<Option<Duration>> for Lifetime {
    fn from(ttl: Option<Duration>) -> Self {
        ttl.map_or(Self::Forever, Self::from)
    }
}

// a write with no precondition has none to fail
impl From<types::Conflict> for Error {
    fn from(_: types::Conflict) -> Self {
        Self::server_error("other", "the host reported a conflict on an unconditional write")
    }
}

fn decode<T: DeserializeOwned>(key: &str, bytes: &[u8]) -> Result<T> {
    serde_json::from_slice(bytes)
        .map_err(|err| Error::server_error("invalid", format!("decoding `{key}`: {err}")))
}

fn encode<T: Serialize + ?Sized>(key: &str, value: &T) -> Result<Vec<u8>> {
    serde_json::to_vec(value)
        .map_err(|err| Error::server_error("invalid", format!("encoding `{key}`: {err}")))
}

fn zero_ttl() -> Error {
    Error::server_error("invalid", "a TTL must be greater than zero")
}

/// The bucket a guest opens when the deployment names no other.
pub const DEFAULT: &str = "default";
