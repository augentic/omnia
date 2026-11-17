use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use omnia_core::{Digest, HostView};
use wasmtime::component::{Access, Accessor, Resource};

use crate::host::generated::omnia::keyvalue::bucket::{Host, HostBucket, HostBucketWithStore};
use crate::host::generated::omnia::keyvalue::types::{
    Conflict, Entry, Lifetime, Precondition, Revision,
};
use crate::host::{Bucket, Error, KeyValueClient, Result, WasiKeyValue};

// Bounds the swaps a write that keeps an entry's lifetime retries while other
// writes land between its read and its swap.
const SWAP_ATTEMPTS: usize = 64;

// The host's side of a guest's `bucket` handle. It keeps the name the guest
// opened for the errors it reports.
#[derive(Clone, Debug)]
pub struct BucketHandle {
    name: String,
    inner: Arc<dyn Bucket>,
}

impl BucketHandle {
    pub fn push<T>(
        accessor: &Accessor<T, WasiKeyValue>, name: String, inner: Arc<dyn Bucket>,
    ) -> Result<Resource<Self>> {
        let handle = Self { name, inner };
        Ok(accessor.with(|mut store| store.get().table.push(handle))?)
    }

    pub fn lookup<T>(
        accessor: &Accessor<T, WasiKeyValue>, resource: &Resource<Self>,
    ) -> Result<Self> {
        Ok(omnia_core::get_cloned(accessor, resource)?)
    }

    pub async fn keys(&self) -> Result<Vec<String>> {
        Ok(self.inner.keys().await?)
    }

    async fn set(
        &self, key: String, value: Vec<u8>, lifetime: Lifetime, when: Option<Precondition>,
    ) -> Result<std::result::Result<Revision, Conflict>> {
        let expiry = Expiry::try_from(lifetime)?;
        let written = revision(&value);
        let swapped = match (when, expiry) {
            (None, expiry) => {
                self.write(key, value, expiry).await?;
                Ok(())
            }
            (Some(Precondition::Absent), Expiry::Replace(ttl) | Expiry::Keep(ttl)) => {
                self.inner.compare_and_swap(key, None, value, ttl).await?
            }
            (Some(Precondition::Matches(expected)), Expiry::Keep(ttl)) => {
                let current = self.inner.get(key.clone()).await?;
                if current.as_deref().map(revision).as_ref() != Some(&expected) {
                    return Ok(Err(Conflict::from(current)));
                }
                self.inner.compare_and_swap(key, current, value, ttl).await?
            }
            (Some(Precondition::Matches(_)), Expiry::Replace(_)) => {
                return Err(self.unsupported("replace an entry's lifetime in a conditional write"));
            }
        };
        Ok(swapped.map(|()| written).map_err(Conflict::from))
    }

    async fn write(&self, key: String, value: Vec<u8>, expiry: Expiry) -> Result<()> {
        match expiry {
            Expiry::Replace(ttl) => Ok(self.inner.set(key, value, ttl).await?),
            Expiry::Keep(ttl) => self.keep(key, value, ttl).await,
        }
    }

    // `Bucket::set` replaces an entry's lifetime and `compare_and_swap` keeps
    // it, so this swaps against the entry it last saw
    async fn keep(&self, key: String, value: Vec<u8>, ttl: Option<Duration>) -> Result<()> {
        let mut current = self.inner.get(key.clone()).await?;
        for _ in 0..SWAP_ATTEMPTS {
            match self.inner.compare_and_swap(key.clone(), current, value.clone(), ttl).await? {
                Ok(()) => return Ok(()),
                Err(observed) => current = observed,
            }
        }
        Err(Error::Unavailable(format!(
            "bucket `{}`: `{key}` changed under each of {SWAP_ATTEMPTS} writes",
            self.name
        )))
    }

    // a call the `Bucket` trait cannot express
    fn unsupported(&self, call: &str) -> Error {
        Error::Unsupported(format!("bucket `{}` cannot {call}", self.name))
    }
}

impl<T> HostBucketWithStore<T> for WasiKeyValue {
    async fn get(
        accessor: &Accessor<T, Self>, self_: Resource<BucketHandle>, key: String,
    ) -> Result<Option<Entry>> {
        let bucket = BucketHandle::lookup(accessor, &self_)?.inner;
        Ok(bucket.get(key).await?.map(Entry::from))
    }

    async fn get_many(
        accessor: &Accessor<T, Self>, self_: Resource<BucketHandle>, keys: Vec<String>,
    ) -> Result<Vec<Option<Entry>>> {
        let bucket = BucketHandle::lookup(accessor, &self_)?.inner;
        let mut entries = Vec::with_capacity(keys.len());
        for key in keys {
            entries.push(bucket.get(key).await?.map(Entry::from));
        }
        Ok(entries)
    }

    async fn exists(
        accessor: &Accessor<T, Self>, self_: Resource<BucketHandle>, key: String,
    ) -> Result<bool> {
        let bucket = BucketHandle::lookup(accessor, &self_)?.inner;
        Ok(bucket.exists(key).await?)
    }

    async fn set(
        accessor: &Accessor<T, Self>, self_: Resource<BucketHandle>, key: String, value: Vec<u8>,
        lifetime: Lifetime, when: Option<Precondition>,
    ) -> Result<std::result::Result<Revision, Conflict>> {
        BucketHandle::lookup(accessor, &self_)?.set(key, value, lifetime, when).await
    }

    async fn set_many(
        accessor: &Accessor<T, Self>, self_: Resource<BucketHandle>,
        entries: Vec<(String, Vec<u8>)>, lifetime: Lifetime,
    ) -> Result<()> {
        let expiry = Expiry::try_from(lifetime)?;
        if let Some(key) = repeated(&entries) {
            return Err(Error::Invalid(format!("`{key}` is given more than once")));
        }
        let bucket = BucketHandle::lookup(accessor, &self_)?;
        for (key, value) in entries {
            bucket.write(key, value, expiry).await?;
        }
        Ok(())
    }

    async fn delete(
        accessor: &Accessor<T, Self>, self_: Resource<BucketHandle>, key: String,
        when: Option<Revision>,
    ) -> Result<std::result::Result<(), Conflict>> {
        let bucket = BucketHandle::lookup(accessor, &self_)?;
        if when.is_some() {
            return Err(bucket.unsupported("check a revision before a delete"));
        }
        bucket.inner.delete(key).await?;
        Ok(Ok(()))
    }

    async fn delete_many(
        accessor: &Accessor<T, Self>, self_: Resource<BucketHandle>, keys: Vec<String>,
    ) -> Result<()> {
        let bucket = BucketHandle::lookup(accessor, &self_)?.inner;
        for key in keys {
            bucket.delete(key).await?;
        }
        Ok(())
    }

    async fn increment(
        accessor: &Accessor<T, Self>, self_: Resource<BucketHandle>, key: String, delta: i64,
        lifetime: Lifetime,
    ) -> Result<i64> {
        let expiry = Expiry::try_from(lifetime)?;
        let bucket = BucketHandle::lookup(accessor, &self_)?;
        match expiry {
            Expiry::Keep(ttl) => Ok(bucket.inner.increment(key, delta, ttl).await?),
            Expiry::Replace(_) => Err(bucket.unsupported("replace a counter's lifetime")),
        }
    }

    fn drop(mut host: Access<'_, T, Self>, rep: Resource<BucketHandle>) -> wasmtime::Result<()> {
        host.get().table.delete(rep)?;
        Ok(())
    }
}

// A `lifetime` with its TTL checked. `Replace` sets the TTL. `Keep` leaves a
// live entry the time it has left and gives a new one the TTL. A zero TTL
// would expire the entry as it is written, so it writes nothing.
#[derive(Clone, Copy)]
enum Expiry {
    Replace(Option<Duration>),
    Keep(Option<Duration>),
}

impl TryFrom<Lifetime> for Expiry {
    type Error = Error;

    fn try_from(lifetime: Lifetime) -> Result<Self> {
        let ttl = |millis: u64| match millis {
            0 => Err(Error::Invalid("a TTL must be greater than zero".to_owned())),
            millis => Ok(Some(Duration::from_millis(millis))),
        };
        Ok(match lifetime {
            Lifetime::Forever => Self::Replace(None),
            Lifetime::TtlMs(millis) => Self::Replace(ttl(millis)?),
            Lifetime::Keep => Self::Keep(None),
            Lifetime::KeepOrTtlMs(millis) => Self::Keep(ttl(millis)?),
        })
    }
}

impl From<Vec<u8>> for Entry {
    fn from(value: Vec<u8>) -> Self {
        Self {
            revision: revision(&value),
            value,
        }
    }
}

impl From<Option<Vec<u8>>> for Conflict {
    fn from(current: Option<Vec<u8>>) -> Self {
        Self {
            current: current.map(Entry::from),
        }
    }
}

// The digest of an entry's bytes stands in for a revision until backends
// carry one, so an entry that changes and changes back matches again.
fn revision(value: &[u8]) -> Revision {
    Digest::of(value).to_string()
}

fn repeated(entries: &[(String, Vec<u8>)]) -> Option<&str> {
    let mut seen = HashSet::new();
    entries.iter().map(|(key, _)| key.as_str()).find(|key| !seen.insert(*key))
}

impl Host for HostView<'_, dyn KeyValueClient> {}

impl HostBucket for HostView<'_, dyn KeyValueClient> {}
