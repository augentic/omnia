//! Key-value state capability.

use std::future::Future;
use std::time::Duration;

use bytes::Bytes;
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::capability::{Error, ErrorKind, Result};

/// Store and retrieve key-value state in a deployment's buckets.
///
/// A handle reads and writes one bucket. A provider's own handle uses
/// [`keyvalue::DEFAULT`](DEFAULT), and [`named`](Self::named) selects
/// another. The deployment declares its buckets, and a call on an undeclared
/// one fails with [`ErrorKind::NotFound`].
///
/// Every provider accepts a portable key, and a provider may reject any
/// other with [`ErrorKind::Invalid`]. A portable key:
///
/// - is not empty;
/// - holds only ASCII letters, digits, `.`, `_`, and `-`;
/// - neither starts nor ends with `.`.
///
/// A TTL is a lifetime the store enforces:
///
/// - A value written without one stays until deleted.
/// - [`set`](Self::set) replaces a value and its lifetime.
/// - [`compare_and_swap`](Self::compare_and_swap) and
///   [`increment`](Self::increment) apply their TTL only when they create the
///   entry. An existing entry keeps the time it has left.
/// - A zero TTL fails with [`ErrorKind::Invalid`] and writes nothing.
///
/// JSON codecs over the stored bytes are [`StateStoreExt`].
#[omnia_guest_macros::capability]
pub trait StateStore: Send + Sync {
    /// The handle [`named`](Self::named) returns.
    type Named: StateStore<Named = Self::Named>;

    /// Retrieve the value at `key`, or `None` when it is absent.
    fn get(&self, key: &str) -> impl Future<Output = Result<Option<Bytes>>> + Send;

    /// Store `value` at `key`, replacing any value and its lifetime.
    fn set(
        &self, key: &str, value: &[u8], ttl: Option<Duration>,
    ) -> impl Future<Output = Result<()>> + Send;

    /// Delete the value at `key`, succeeding when it is already absent.
    fn delete(&self, key: &str) -> impl Future<Output = Result<()>> + Send;

    /// Replace the value at `key` with `new` while it still equals `current`.
    ///
    /// A `current` of `None` means the key is absent, so the swap creates
    /// the entry. A mismatch changes nothing and returns the value it found
    /// as a [`Conflict`]. There is no retry loop.
    fn compare_and_swap(
        &self, key: &str, current: Option<&[u8]>, new: &[u8], ttl: Option<Duration>,
    ) -> impl Future<Output = Result<Result<(), Conflict>>> + Send;

    /// Atomically add `delta` to the counter at `key` and return its new value.
    ///
    /// An absent counter starts at zero. A counter's encoding belongs to the
    /// store, so it is read with a `delta` of zero, never with
    /// [`get`](Self::get).
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
    /// Stored bytes that do not decode as `T` fail with
    /// [`ErrorKind::Invalid`].
    fn get_json<T: DeserializeOwned>(
        &self, key: &str,
    ) -> impl Future<Output = Result<Option<T>>> + Send {
        async move {
            let Some(bytes) = self.get(key).await? else {
                return Ok(None);
            };
            serde_json::from_slice(&bytes).map(Some).map_err(|err| {
                Error::new(ErrorKind::Invalid, format!("decoding `{key}`")).with_source(err)
            })
        }
    }

    /// Serialize `value` as JSON and store it at `key`.
    ///
    /// A value that does not serialize fails with [`ErrorKind::Invalid`].
    fn set_json<T: Serialize + Sync + ?Sized>(
        &self, key: &str, value: &T, ttl: Option<Duration>,
    ) -> impl Future<Output = Result<()>> + Send {
        async move {
            let bytes = serde_json::to_vec(value).map_err(|err| {
                Error::new(ErrorKind::Invalid, format!("encoding `{key}`")).with_source(err)
            })?;
            self.set(key, &bytes, ttl).await
        }
    }
}

impl<P: StateStore + ?Sized> StateStoreExt for P {}

/// The bucket a provider's own [`StateStore`] handle uses.
pub const DEFAULT: &str = "default";

/// The value a [`compare_and_swap`](StateStore::compare_and_swap) found in
/// place of the one it expected.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Conflict {
    /// The value stored at the key, or `None` when it is absent.
    pub current: Option<Bytes>,
}
