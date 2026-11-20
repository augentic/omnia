//! `omnia:keyvalue`, the buckets of key-value state a deployment declares.
//!
//! A guest opens a [`Bucket`] with [`open`], or the [`DEFAULT`] bucket with
//! [`default`]. Beside the generated methods, a bucket has unconditional
//! forms that take borrowed keys and values:
//!
//! - [`read`](Bucket::read), [`write`](Bucket::write), and
//!   [`remove`](Bucket::remove) a value's bytes, and
//!   [`contains`](Bucket::contains) tells whether one is stored.
//! - [`get_json`](Bucket::get_json) and [`set_json`](Bucket::set_json) a
//!   value as JSON.
//! - [`add`](Bucket::add) to a counter.
//! - [`keys`](Bucket::keys) lists one page of keys.
//!
//! A conditional write lands only while the entry is still at the
//! [`Revision`] its caller read, and otherwise answers a [`Conflict`] holding
//! the entry it found:
//!
//! - [`compare_and_swap`](Bucket::compare_and_swap) writes bytes.
//! - [`get_json_entry`](Bucket::get_json_entry) reads a JSON value with its
//!   revision, and [`set_json_if`](Bucket::set_json_if) writes one under a
//!   [`Precondition`].
//! - [`update_json`](Bucket::update_json) runs a closure over a JSON value
//!   until its write lands.
//!
//! A conditional write's TTL applies only to an entry it creates, so an
//! existing entry keeps the time it has left. An unconditional write replaces
//! the entry's lifetime. The generated `get`, `set`, and `delete` take a
//! [`Precondition`] too, and a TTL becomes their [`Lifetime`] through `From`.
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

pub use crate::bindings::imports::omnia::keyvalue::*;
use crate::error::unconditional;
pub use crate::keyvalue::bucket::Bucket;
pub use crate::keyvalue::keys::Page;
pub use crate::keyvalue::types::{Conflict, Entry, Lifetime, Precondition, Revision};
use crate::{Error, Result};

/// The bucket a guest opens when the deployment names no other.
pub const DEFAULT: &str = "default";

/// Open the bucket `name`, a binding the deployment declares.
///
/// # Errors
///
/// The host's failure, under its code: `not-found` for a bucket the
/// deployment does not declare.
pub async fn open(name: &str) -> Result<Bucket> {
    Ok(store::open(name.to_owned()).await?)
}

/// Open the [`DEFAULT`] bucket.
///
/// # Errors
///
/// As [`open`].
pub async fn default() -> Result<Bucket> {
    open(DEFAULT).await
}

impl Bucket {
    /// The value at `key`, or `None` when it is absent.
    ///
    /// # Errors
    ///
    /// The host's failure, under its code.
    pub async fn read(&self, key: &str) -> Result<Option<Vec<u8>>> {
        Ok(self.get(key.to_owned()).await?.map(|entry| entry.value))
    }

    /// Whether an entry is stored at `key`.
    ///
    /// # Errors
    ///
    /// The host's failure, under its code.
    pub async fn contains(&self, key: &str) -> Result<bool> {
        Ok(self.exists(key.to_owned()).await?)
    }

    /// Store `value` at `key`, replacing any entry and its lifetime.
    ///
    /// A `ttl` of `None` keeps the value until it is deleted.
    ///
    /// # Errors
    ///
    /// The host's failure, under its code: `invalid` for a zero `ttl`.
    pub async fn write(&self, key: &str, value: &[u8], ttl: Option<Duration>) -> Result<()> {
        unconditional(self.set(key.to_owned(), value.to_vec(), ttl.into(), None).await?)?;
        Ok(())
    }

    /// Delete the entry at `key`. Deleting an absent key succeeds.
    ///
    /// # Errors
    ///
    /// The host's failure, under its code.
    pub async fn remove(&self, key: &str) -> Result<()> {
        unconditional(self.delete(key.to_owned(), None).await?)
    }

    /// Add `delta` to the counter at `key` and return its new value.
    ///
    /// An absent counter starts at zero. One this call creates expires after
    /// `ttl`, or never for `None`, and an existing counter keeps the time it
    /// has left.
    ///
    /// # Errors
    ///
    /// The host's failure, under its code: `invalid` for a zero `ttl`, a
    /// value at `key` that is not a counter, or a sum outside an `i64`.
    pub async fn add(&self, key: &str, delta: i64, ttl: Option<Duration>) -> Result<i64> {
        Ok(self.increment(key.to_owned(), delta, ttl.map(ttl_ms)).await?)
    }

    /// One page of the keys that start with `prefix`, as [`keys::list`]
    /// pages them.
    ///
    /// `after` is the [`next`](Page::next) of the previous page of the same
    /// listing, and a page holds at most `limit` keys.
    ///
    /// # Errors
    ///
    /// The host's failure, under its code: `invalid` for a zero `limit`.
    pub async fn keys(
        &self, prefix: &str, after: Option<&str>, limit: Option<u32>,
    ) -> Result<Page> {
        Ok(keys::list(self, prefix.to_owned(), after.map(str::to_owned), limit).await?)
    }

    /// The JSON value at `key` decoded as `T`, or `None` when it is absent.
    ///
    /// # Errors
    ///
    /// - [`ServerError`](crate::ErrorKind::ServerError) with the code `invalid` when the
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
    /// - [`ServerError`](crate::ErrorKind::ServerError) with the code `invalid` when
    ///   `value` does not serialize, or `ttl` is zero.
    /// - The host's failure, under its code.
    pub async fn set_json<T: Serialize + Sync + ?Sized>(
        &self, key: &str, value: &T, ttl: Option<Duration>,
    ) -> Result<()> {
        let bytes = encode(key, value)?;
        unconditional(self.set(key.to_owned(), bytes, ttl.into(), None).await?)?;
        Ok(())
    }

    /// The JSON value at `key` decoded as `T`, with the [`Revision`] it is
    /// stored at, or `None` when it is absent.
    ///
    /// # Errors
    ///
    /// As [`get_json`](Self::get_json).
    pub async fn get_json_entry<T: DeserializeOwned>(
        &self, key: &str,
    ) -> Result<Option<(T, Revision)>> {
        let entry = self.get(key.to_owned()).await?;
        entry.map(|entry| Ok((decode(key, &entry.value)?, entry.revision))).transpose()
    }

    /// Store `value` at `key` as JSON while `when` holds, and return the
    /// [`Revision`] it is stored at.
    ///
    /// When `when` does not hold, nothing changes and the [`Conflict`] holds
    /// the entry found. A new entry expires after `ttl`, or never for `None`,
    /// and an existing one keeps the time it has left.
    ///
    /// # Errors
    ///
    /// - [`ServerError`](crate::ErrorKind::ServerError) with the code `invalid` when
    ///   `value` does not serialize, or `ttl` is zero.
    /// - The host's failure, under its code.
    pub async fn set_json_if<T: Serialize + Sync + ?Sized>(
        &self, key: &str, value: &T, ttl: Option<Duration>, when: Precondition,
    ) -> Result<Result<Revision, Conflict>> {
        let bytes = encode(key, value)?;
        Ok(self.set(key.to_owned(), bytes, Lifetime::keeping(ttl), Some(when)).await?)
    }

    /// Replace the JSON value at `key` with what `update` makes of it, and
    /// return the value stored.
    ///
    /// `update` takes the value at `key`, or `None` when it is absent. It
    /// returns the value to store, or an error that ends the update with
    /// nothing changed. When another write lands between the read and the
    /// store, `update` runs again over the value that write stored, so it may
    /// run more than once. A new entry expires after `ttl`, or never for
    /// `None`, and an existing one keeps the time it has left.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use omnia_sdk::keyvalue::Bucket;
    /// use omnia_sdk::not_found;
    /// use serde::{Deserialize, Serialize};
    ///
    /// #[derive(Serialize, Deserialize)]
    /// struct Profile {
    ///     name: String,
    ///     logins: u32,
    /// }
    ///
    /// # async fn login(users: &Bucket) -> omnia_sdk::Result<()> {
    /// let profile = users
    ///     .update_json("user.ann", None, |profile: Option<Profile>| {
    ///         let mut profile = profile.ok_or_else(|| not_found!("no user `ann`"))?;
    ///         profile.logins += 1;
    ///         Ok(profile)
    ///     })
    ///     .await?;
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// - The error `update` returns.
    /// - [`ServerError`](crate::ErrorKind::ServerError) with the code `invalid` when a
    ///   stored value does not decode as `T`, the new one does not serialize,
    ///   or `ttl` is zero.
    /// - The host's failure, under its code.
    pub async fn update_json<T, F>(
        &self, key: &str, ttl: Option<Duration>, mut update: F,
    ) -> Result<T>
    where
        T: Serialize + DeserializeOwned,
        F: FnMut(Option<T>) -> Result<T>,
    {
        let mut found = self.get(key.to_owned()).await?;
        loop {
            let (current, when) = match found {
                Some(entry) => {
                    (Some(decode(key, &entry.value)?), Precondition::Matches(entry.revision))
                }
                None => (None, Precondition::Absent),
            };
            let next = update(current)?;
            let bytes = encode(key, &next)?;
            match self.set(key.to_owned(), bytes, Lifetime::keeping(ttl), Some(when)).await? {
                Ok(_revision) => return Ok(next),
                Err(conflict) => found = conflict.current,
            }
        }
    }

    /// Replace the value at `key` with `new` while the entry is still at the
    /// revision `current`, and return the revision `new` is stored at.
    ///
    /// A `current` of `None` means the key is absent, so the swap creates the
    /// entry. A mismatch changes nothing and returns the entry it found, and
    /// there is no retry loop. A new entry expires after `ttl`, or never for
    /// `None`, and an existing one keeps the time it has left.
    ///
    /// # Errors
    ///
    /// The host's failure, under its code: `invalid` for a zero `ttl`.
    pub async fn compare_and_swap(
        &self, key: &str, current: Option<Revision>, new: &[u8], ttl: Option<Duration>,
    ) -> Result<Result<Revision, Conflict>> {
        let when = current.map_or(Precondition::Absent, Precondition::Matches);
        Ok(self.set(key.to_owned(), new.to_vec(), Lifetime::keeping(ttl), Some(when)).await?)
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

impl Lifetime {
    // a conditional write's TTL applies only to an entry it creates
    fn keeping(ttl: Option<Duration>) -> Self {
        ttl.map_or(Self::Keep, |ttl| Self::KeepOrTtlMs(ttl_ms(ttl)))
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
