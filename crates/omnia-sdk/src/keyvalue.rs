//! Key-value wire types.

#[cfg(all(target_arch = "wasm32", target_os = "wasi"))]
use std::time::Duration;

use bytes::Bytes;
#[cfg(all(target_arch = "wasm32", target_os = "wasi"))]
use omnia_wasi_keyvalue::omnia::store;

#[cfg(all(target_arch = "wasm32", target_os = "wasi"))]
use crate::StateStore;
#[cfg(all(target_arch = "wasm32", target_os = "wasi"))]
use crate::capability::{Error, ErrorKind, Result};

/// The bucket a provider's own [`StateStore`](crate::StateStore) handle uses.
pub const DEFAULT: &str = "default";

/// The value a [`compare_and_swap`](crate::StateStore::compare_and_swap)
/// found in place of the one it expected.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Conflict {
    /// The value stored at the key, or `None` when it is absent.
    pub current: Option<Bytes>,
}

/// A [`StateStore`](crate::StateStore) handle on one bucket of the WASI host.
///
/// [`StateStore::named`](crate::StateStore::named) on
/// [`WasiProvider`](crate::WasiProvider) returns one.
#[cfg(all(target_arch = "wasm32", target_os = "wasi"))]
#[derive(Clone, Debug)]
pub struct Handle {
    name: String,
}

#[cfg(all(target_arch = "wasm32", target_os = "wasi"))]
impl Handle {
    pub(crate) fn new(name: &str) -> Self {
        Self {
            name: name.to_owned(),
        }
    }
}

#[cfg(all(target_arch = "wasm32", target_os = "wasi"))]
impl StateStore for Handle {
    type Named = Self;

    async fn get(&self, key: &str) -> Result<Option<Bytes>> {
        Ok(store::get(self.name.clone(), key.to_owned()).await?.map(Bytes::from))
    }

    async fn set(&self, key: &str, value: &[u8], ttl: Option<Duration>) -> Result<()> {
        let ttl = ttl.map(millis);
        Ok(store::set(self.name.clone(), key.to_owned(), value.to_vec(), ttl).await?)
    }

    async fn delete(&self, key: &str) -> Result<()> {
        Ok(store::delete(self.name.clone(), key.to_owned()).await?)
    }

    async fn compare_and_swap(
        &self, key: &str, current: Option<&[u8]>, new: &[u8], ttl: Option<Duration>,
    ) -> Result<Result<(), Conflict>> {
        let (current, ttl) = (current.map(<[u8]>::to_vec), ttl.map(millis));
        let swapped =
            store::compare_and_swap(self.name.clone(), key.to_owned(), current, new.to_vec(), ttl)
                .await?;
        Ok(swapped.map_err(|conflict| Conflict {
            current: conflict.current.map(Bytes::from),
        }))
    }

    async fn increment(&self, key: &str, delta: i64, ttl: Option<Duration>) -> Result<i64> {
        let ttl = ttl.map(millis);
        Ok(store::increment(self.name.clone(), key.to_owned(), delta, ttl).await?)
    }

    fn named(&self, name: &str) -> Self {
        Self::new(name)
    }
}

// rounds up, so a sub-millisecond TTL still expires and only `Duration::ZERO`
// sends the zero the host rejects
#[cfg(all(target_arch = "wasm32", target_os = "wasi"))]
fn millis(ttl: Duration) -> u64 {
    u64::try_from(ttl.as_nanos().div_ceil(1_000_000)).unwrap_or(u64::MAX)
}

#[cfg(all(target_arch = "wasm32", target_os = "wasi"))]
impl From<store::Error> for Error {
    fn from(err: store::Error) -> Self {
        let (kind, message) = match err {
            store::Error::NotFound(message) => (ErrorKind::NotFound, message),
            store::Error::Denied(message) => (ErrorKind::Denied, message),
            store::Error::Invalid(message) => (ErrorKind::Invalid, message),
            store::Error::Unsupported(message) => (ErrorKind::Unsupported, message),
            store::Error::Unavailable(message) => (ErrorKind::Unavailable, message),
            store::Error::Other(message) => (ErrorKind::Other, message),
        };
        Self::new(kind, message)
    }
}
