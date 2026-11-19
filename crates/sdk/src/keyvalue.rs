//! Key-value state capability.

use std::future::Future;
use std::time::Duration;

use bytes::Bytes;
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::{Error, Result};

/// Store and retrieve key-value state in a deployment's buckets.
///
/// A handle reads and writes one bucket. A provider's own handle uses
/// [`keyvalue::DEFAULT`](DEFAULT), and [`named`](Self::named) selects
/// another. The deployment declares its buckets, and a call on an undeclared
/// one fails with the code `not-found`.
///
/// Every provider accepts a portable key, and a provider may reject any
/// other with the code `invalid`. A portable key:
///
/// - is one or more segments joined by `.`;
/// - has no empty segment, and holds only ASCII letters, digits, `_`, and
///   `-` in each;
/// - is at most 255 bytes long.
///
/// A TTL is a lifetime the store enforces:
///
/// - A value written without one stays until deleted.
/// - [`set`](Self::set) replaces a value and its lifetime.
/// - [`set_if`](Self::set_if), [`compare_and_swap`](Self::compare_and_swap),
///   and [`increment`](Self::increment) apply their TTL only when they create
///   the entry. An existing entry keeps the time it has left.
/// - A zero TTL fails with the code `invalid` and writes nothing.
///
/// JSON codecs over the stored bytes are [`StateStoreExt`].
#[omnia_guest_macros::capability]
pub trait StateStore: Send + Sync {
    /// The handle [`named`](Self::named) returns.
    type Named: StateStore<Named = Self::Named>;

    /// Retrieve the value at `key`, or `None` when it is absent.
    fn get(&self, key: &str) -> impl Future<Output = Result<Option<Bytes>>> + Send;

    /// Retrieve the value at `key` with its [`Revision`], or `None` when it
    /// is absent.
    fn get_entry(&self, key: &str) -> impl Future<Output = Result<Option<Entry>>> + Send;

    /// Store `value` at `key`, replacing any value and its lifetime.
    fn set(
        &self, key: &str, value: &[u8], ttl: Option<Duration>,
    ) -> impl Future<Output = Result<()>> + Send;

    /// Store `value` at `key` while `when` holds, and return the revision
    /// written.
    ///
    /// A precondition that does not hold changes nothing and returns the
    /// entry it found, or `None` when the key is absent. There is no retry
    /// loop.
    fn set_if(
        &self, key: &str, value: &[u8], ttl: Option<Duration>, when: Precondition,
    ) -> impl Future<Output = Result<Result<Revision, Option<Entry>>>> + Send;

    /// Delete the value at `key`, succeeding when it is already absent.
    fn delete(&self, key: &str) -> impl Future<Output = Result<()>> + Send;

    /// Replace the value at `key` with `new` while it still equals `current`.
    ///
    /// A `current` of `None` means the key is absent, so the swap creates
    /// the entry. A mismatch changes nothing and returns the value it found
    /// as a [`Conflict`]. There is no retry loop.
    fn compare_and_swap(
        &self, key: &str, current: Option<&[u8]>, new: &[u8], ttl: Option<Duration>,
    ) -> impl Future<Output = Result<Result<(), Conflict>>> + Send {
        async move {
            // refuse a zero TTL before a mismatch can answer
            if ttl == Some(Duration::ZERO) {
                return Err(Error::server_error("invalid", "a TTL must be greater than zero"));
            }

            // write under the revision the expected bytes are stored at
            let when = match current {
                None => Precondition::Absent,
                Some(expected) => match self.get_entry(key).await? {
                    Some(entry) if entry.value == expected => Precondition::Matches(entry.revision),
                    found => return Ok(Err(Conflict::found(found))),
                },
            };
            let swapped = self.set_if(key, new, ttl, when).await?;
            Ok(swapped.map(|_revision| ()).map_err(Conflict::found))
        }
    }

    /// Atomically add `delta` to the counter at `key` and return its new value.
    ///
    /// An absent counter starts at zero. A counter is stored as decimal
    /// text, so [`get`](Self::get) reads one and [`set`](Self::set) writes
    /// one. A value that is not a counter, or a sum that overflows an `i64`,
    /// fails with the code `invalid` and changes nothing.
    fn increment(
        &self, key: &str, delta: i64, ttl: Option<Duration>,
    ) -> impl Future<Output = Result<i64>> + Send;

    /// A handle to the bucket `name` on the same provider.
    fn named(&self, name: &str) -> Self::Named;
}

/// JSON codecs over any [`StateStore`].
///
/// Blanket-implemented for every [`StateStore`]; never implemented by hand.
pub trait StateStoreExt: StateStore {
    /// Retrieve and deserialize the JSON value at `key`.
    ///
    /// Stored bytes that do not decode as `T` fail with the code `invalid`.
    fn get_json<T: DeserializeOwned>(
        &self, key: &str,
    ) -> impl Future<Output = Result<Option<T>>> + Send {
        async move {
            let Some(bytes) = self.get(key).await? else {
                return Ok(None);
            };
            serde_json::from_slice(&bytes)
                .map(Some)
                .map_err(|err| Error::server_error("invalid", format!("decoding `{key}`: {err}")))
        }
    }

    /// Serialize `value` as JSON and store it at `key`.
    ///
    /// A value that does not serialize fails with the code `invalid`.
    fn set_json<T: Serialize + Sync + ?Sized>(
        &self, key: &str, value: &T, ttl: Option<Duration>,
    ) -> impl Future<Output = Result<()>> + Send {
        async move {
            let bytes = serde_json::to_vec(value).map_err(|err| {
                Error::server_error("invalid", format!("encoding `{key}`: {err}"))
            })?;
            self.set(key, &bytes, ttl).await
        }
    }
}

impl<P: StateStore + ?Sized> StateStoreExt for P {}

/// The bucket a provider's own [`StateStore`] handle uses.
pub const DEFAULT: &str = "default";

/// A stored value and the revision of the state that holds it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// The stored bytes.
    pub value: Bytes,
    /// The revision of the stored state.
    pub revision: Revision,
}

/// A token for one stored state of an entry, compared only for equality.
///
/// It changes whenever the entry's bytes change. Two states that hold the
/// same bytes may share one, so an entry that changed and changed back may
/// still match.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Revision(String);

impl Revision {
    /// The token as the store reports it.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<String> for Revision {
    fn from(token: String) -> Self {
        Self(token)
    }
}

impl From<Revision> for String {
    fn from(revision: Revision) -> Self {
        revision.0
    }
}

/// What must hold for a [`set_if`](StateStore::set_if) to take effect.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Precondition {
    /// No entry is stored at the key.
    Absent,
    /// The entry stored at the key has this revision.
    Matches(Revision),
}

/// The value a [`compare_and_swap`](StateStore::compare_and_swap) found in
/// place of the one it expected.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Conflict {
    /// The value stored at the key, or `None` when it is absent.
    pub current: Option<Bytes>,
}

impl Conflict {
    fn found(current: Option<Entry>) -> Self {
        Self {
            current: current.map(|entry| entry.value),
        }
    }
}
