//! Key-value state capability.

use std::future::Future;
use std::time::Duration;

use bytes::Bytes;
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::capability::{Error, ErrorKind, Result};
use crate::keyvalue::Conflict;
#[cfg(all(target_arch = "wasm32", target_os = "wasi"))]
use crate::keyvalue::{DEFAULT, Handle};

/// Store and retrieve key-value state in a deployment's buckets.
///
/// A handle reads and writes one bucket. A provider's own handle uses
/// [`keyvalue::DEFAULT`](crate::keyvalue::DEFAULT), and
/// [`named`](Self::named) selects another. The deployment declares its
/// buckets, and a call on an undeclared one fails with
/// [`ErrorKind::NotFound`].
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
    type Named: StateStore;

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

#[cfg(all(target_arch = "wasm32", target_os = "wasi"))]
impl StateStore for crate::WasiProvider {
    type Named = Handle;

    async fn get(&self, key: &str) -> Result<Option<Bytes>> {
        Handle::new(DEFAULT).get(key).await
    }

    async fn set(&self, key: &str, value: &[u8], ttl: Option<Duration>) -> Result<()> {
        Handle::new(DEFAULT).set(key, value, ttl).await
    }

    async fn delete(&self, key: &str) -> Result<()> {
        Handle::new(DEFAULT).delete(key).await
    }

    async fn compare_and_swap(
        &self, key: &str, current: Option<&[u8]>, new: &[u8], ttl: Option<Duration>,
    ) -> Result<Result<(), Conflict>> {
        Handle::new(DEFAULT).compare_and_swap(key, current, new, ttl).await
    }

    async fn increment(&self, key: &str, delta: i64, ttl: Option<Duration>) -> Result<i64> {
        Handle::new(DEFAULT).increment(key, delta, ttl).await
    }

    fn named(&self, name: &str) -> Handle {
        Handle::new(name)
    }
}
