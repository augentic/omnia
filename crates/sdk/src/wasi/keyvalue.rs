//! [`StateStore`] over `omnia:keyvalue`.

use std::borrow::Cow;
use std::time::Duration;

use bytes::Bytes;
use omnia_wasi_keyvalue::omnia::store;

use crate::keyvalue::{Conflict, DEFAULT};
use crate::{Error, HasStateStore, Result, StateStore, WasiProvider};

static DEFAULT_HANDLE: Handle = Handle {
    name: Cow::Borrowed(DEFAULT),
};

impl HasStateStore for WasiProvider {
    type Inner = Handle;

    fn state_store(&self) -> &Handle {
        &DEFAULT_HANDLE
    }
}

/// A [`StateStore`] handle on one bucket of the WASI host.
///
/// [`StateStore::named`] on [`WasiProvider`] returns one.
#[derive(Clone, Debug)]
pub struct Handle {
    name: Cow<'static, str>,
}

impl StateStore for Handle {
    type Named = Self;

    async fn get(&self, key: &str) -> Result<Option<Bytes>> {
        Ok(store::get(self.name.to_string(), key.to_owned()).await?.map(Bytes::from))
    }

    async fn set(&self, key: &str, value: &[u8], ttl: Option<Duration>) -> Result<()> {
        let ttl = ttl.map(millis);
        Ok(store::set(self.name.to_string(), key.to_owned(), value.to_vec(), ttl).await?)
    }

    async fn delete(&self, key: &str) -> Result<()> {
        Ok(store::delete(self.name.to_string(), key.to_owned()).await?)
    }

    async fn compare_and_swap(
        &self, key: &str, current: Option<&[u8]>, new: &[u8], ttl: Option<Duration>,
    ) -> Result<Result<(), Conflict>> {
        let (current, ttl) = (current.map(<[u8]>::to_vec), ttl.map(millis));
        let swapped = store::compare_and_swap(
            self.name.to_string(),
            key.to_owned(),
            current,
            new.to_vec(),
            ttl,
        )
        .await?;
        Ok(swapped.map_err(|conflict| Conflict {
            current: conflict.current.map(Bytes::from),
        }))
    }

    async fn increment(&self, key: &str, delta: i64, ttl: Option<Duration>) -> Result<i64> {
        let ttl = ttl.map(millis);
        Ok(store::increment(self.name.to_string(), key.to_owned(), delta, ttl).await?)
    }

    fn named(&self, name: &str) -> Self {
        Self {
            name: Cow::Owned(name.to_owned()),
        }
    }
}

// rounds up, so a sub-millisecond TTL still expires and only `Duration::ZERO`
// sends the zero the host rejects
fn millis(ttl: Duration) -> u64 {
    u64::try_from(ttl.as_nanos().div_ceil(1_000_000)).unwrap_or(u64::MAX)
}

impl From<store::Error> for Error {
    fn from(err: store::Error) -> Self {
        match err {
            store::Error::NotFound(message) => Self::server_error("not-found", message),
            store::Error::Denied(message) => Self::server_error("denied", message),
            store::Error::Invalid(message) => Self::server_error("invalid", message),
            store::Error::Unsupported(message) => Self::server_error("unsupported", message),
            store::Error::Unavailable(message) => Self::bad_gateway("unavailable", message),
            store::Error::Other(message) => Self::server_error("other", message),
        }
    }
}
