//! [`StateStore`] over `omnia:keyvalue`.

use std::time::Duration;

use bytes::Bytes;
use omnia_wasi_keyvalue::omnia::store;

use crate::capability::{Error, ErrorKind, Result};
use crate::keyvalue::{Conflict, DEFAULT};
use crate::{StateStore, WasiProvider};

impl StateStore for WasiProvider {
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

/// A [`StateStore`] handle on one bucket of the WASI host.
///
/// [`StateStore::named`] on [`WasiProvider`] returns one.
#[derive(Clone, Debug)]
pub struct Handle {
    name: String,
}

impl Handle {
    fn new(name: &str) -> Self {
        Self {
            name: name.to_owned(),
        }
    }
}

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
fn millis(ttl: Duration) -> u64 {
    u64::try_from(ttl.as_nanos().div_ceil(1_000_000)).unwrap_or(u64::MAX)
}

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
