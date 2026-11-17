//! [`StateStore`] over `omnia:keyvalue`.

use std::borrow::Cow;
use std::time::Duration;

use bytes::Bytes;
use omnia_wasi_keyvalue::omnia::bucket::Bucket;
use omnia_wasi_keyvalue::omnia::store;
use omnia_wasi_keyvalue::omnia::types::{self, Lifetime, Precondition};

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

impl Handle {
    async fn open(&self) -> Result<Bucket> {
        Ok(store::open(self.name.to_string()).await?)
    }
}

impl StateStore for Handle {
    type Named = Self;

    async fn get(&self, key: &str) -> Result<Option<Bytes>> {
        let entry = self.open().await?.get(key.to_owned()).await?;
        Ok(entry.map(|entry| Bytes::from(entry.value)))
    }

    async fn set(&self, key: &str, value: &[u8], ttl: Option<Duration>) -> Result<()> {
        let lifetime = ttl.map_or(Lifetime::Forever, |ttl| Lifetime::TtlMs(millis(ttl)));
        self.open().await?.set(key.to_owned(), value.to_vec(), lifetime, None).await??;
        Ok(())
    }

    async fn delete(&self, key: &str) -> Result<()> {
        self.open().await?.delete(key.to_owned(), None).await??;
        Ok(())
    }

    async fn compare_and_swap(
        &self, key: &str, current: Option<&[u8]>, new: &[u8], ttl: Option<Duration>,
    ) -> Result<Result<(), Conflict>> {
        let bucket = self.open().await?;

        // match the expected bytes to the revision they are stored at, refusing
        // a zero TTL first, since a mismatch never reaches the host
        let when = match current {
            None => Precondition::Absent,
            Some(expected) => {
                if ttl == Some(Duration::ZERO) {
                    return Err(Error::server_error("invalid", "a TTL must be greater than zero"));
                }
                match bucket.get(key.to_owned()).await? {
                    Some(entry) if entry.value == expected => Precondition::Matches(entry.revision),
                    current => return Ok(Err(types::Conflict { current }.into())),
                }
            }
        };

        let swapped = bucket.set(key.to_owned(), new.to_vec(), on_create(ttl), Some(when)).await?;
        Ok(swapped.map(|_revision| ()).map_err(Conflict::from))
    }

    async fn increment(&self, key: &str, delta: i64, ttl: Option<Duration>) -> Result<i64> {
        Ok(self.open().await?.increment(key.to_owned(), delta, on_create(ttl)).await?)
    }

    fn named(&self, name: &str) -> Self {
        Self {
            name: Cow::Owned(name.to_owned()),
        }
    }
}

// an existing entry keeps the time it has left
fn on_create(ttl: Option<Duration>) -> Lifetime {
    ttl.map_or(Lifetime::Keep, |ttl| Lifetime::KeepOrTtlMs(millis(ttl)))
}

// rounds up, so a sub-millisecond TTL still expires and only `Duration::ZERO`
// sends the zero the host rejects
fn millis(ttl: Duration) -> u64 {
    u64::try_from(ttl.as_nanos().div_ceil(1_000_000)).unwrap_or(u64::MAX)
}

impl From<types::Conflict> for Conflict {
    fn from(conflict: types::Conflict) -> Self {
        Self {
            current: conflict.current.map(|entry| Bytes::from(entry.value)),
        }
    }
}

impl From<types::Error> for Error {
    fn from(err: types::Error) -> Self {
        match err {
            types::Error::NotFound(message) => Self::server_error("not-found", message),
            types::Error::Denied(message) => Self::server_error("denied", message),
            types::Error::Invalid(message) => Self::server_error("invalid", message),
            types::Error::Unsupported(message) => Self::server_error("unsupported", message),
            types::Error::Unavailable(message) => Self::bad_gateway("unavailable", message),
            types::Error::Other(message) => Self::server_error("other", message),
        }
    }
}

// a write with no precondition has none to fail
impl From<types::Conflict> for Error {
    fn from(_: types::Conflict) -> Self {
        Self::server_error("other", "the host reported a conflict on an unconditional write")
    }
}
