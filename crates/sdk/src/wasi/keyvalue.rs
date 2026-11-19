//! [`StateStore`] over `omnia:keyvalue`.

use std::borrow::Cow;
use std::time::Duration;

use bytes::Bytes;
use omnia_wasi_keyvalue::omnia::bucket::Bucket;
use omnia_wasi_keyvalue::omnia::store;
use omnia_wasi_keyvalue::omnia::types::{self, Lifetime};

use crate::keyvalue::{DEFAULT, Entry, Precondition, Revision};
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

    async fn get_entry(&self, key: &str) -> Result<Option<Entry>> {
        let entry = self.open().await?.get(key.to_owned()).await?;
        Ok(entry.map(Entry::from))
    }

    async fn set(&self, key: &str, value: &[u8], ttl: Option<Duration>) -> Result<()> {
        let lifetime = ttl.map_or(Lifetime::Forever, |ttl| Lifetime::TtlMs(millis(ttl)));
        self.open().await?.set(key.to_owned(), value.to_vec(), lifetime, None).await??;
        Ok(())
    }

    async fn set_if(
        &self, key: &str, value: &[u8], ttl: Option<Duration>, when: Precondition,
    ) -> Result<Result<Revision, Option<Entry>>> {
        let lifetime = ttl.map_or(Lifetime::Keep, |ttl| Lifetime::KeepOrTtlMs(millis(ttl)));
        let when = Some(types::Precondition::from(when));
        let written =
            self.open().await?.set(key.to_owned(), value.to_vec(), lifetime, when).await?;
        Ok(written.map(Revision::from).map_err(|conflict| conflict.current.map(Entry::from)))
    }

    async fn delete(&self, key: &str) -> Result<()> {
        self.open().await?.delete(key.to_owned(), None).await??;
        Ok(())
    }

    async fn increment(&self, key: &str, delta: i64, ttl: Option<Duration>) -> Result<i64> {
        Ok(self.open().await?.increment(key.to_owned(), delta, ttl.map(millis)).await?)
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

impl From<types::Entry> for Entry {
    fn from(entry: types::Entry) -> Self {
        Self {
            value: Bytes::from(entry.value),
            revision: Revision::from(entry.revision),
        }
    }
}

impl From<Precondition> for types::Precondition {
    fn from(when: Precondition) -> Self {
        match when {
            Precondition::Absent => Self::Absent,
            Precondition::Matches(revision) => Self::Matches(revision.into()),
        }
    }
}

// a write with no precondition has none to fail
impl From<types::Conflict> for Error {
    fn from(_: types::Conflict) -> Self {
        Self::server_error("other", "the host reported a conflict on an unconditional write")
    }
}
