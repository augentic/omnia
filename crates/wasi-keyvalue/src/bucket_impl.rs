use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use omnia_core::HostView;
use wasmtime::component::{Access, Accessor, Resource};

use crate::host::generated::omnia::keyvalue::bucket::{Host, HostBucket, HostBucketWithStore};
use crate::host::generated::omnia::keyvalue::types;
use crate::host::{
    Bucket, Entry, Error, KeyValueClient, Lifetime, Precondition, Result, Revision, WasiKeyValue,
};

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

    // a batch verb has no `conflict` to answer with, and its writes carry no
    // precondition that could fail
    fn conflicted(&self, key: &str) -> Error {
        Error::Other(format!(
            "bucket `{}` reported a conflict on `{key}` for a write with no precondition",
            self.name
        ))
    }
}

impl<T> HostBucketWithStore<T> for WasiKeyValue {
    async fn get(
        accessor: &Accessor<T, Self>, self_: Resource<BucketHandle>, key: String,
    ) -> Result<Option<types::Entry>> {
        let bucket = BucketHandle::lookup(accessor, &self_)?.inner;
        Ok(bucket.get(key).await?.map(types::Entry::from))
    }

    async fn get_many(
        accessor: &Accessor<T, Self>, self_: Resource<BucketHandle>, keys: Vec<String>,
    ) -> Result<Vec<Option<types::Entry>>> {
        let bucket = BucketHandle::lookup(accessor, &self_)?.inner;
        let mut entries = Vec::with_capacity(keys.len());
        for key in keys {
            entries.push(bucket.get(key).await?.map(types::Entry::from));
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
        lifetime: types::Lifetime, when: Option<types::Precondition>,
    ) -> Result<std::result::Result<types::Revision, types::Conflict>> {
        let lifetime = Lifetime::try_from(lifetime)?;
        let bucket = BucketHandle::lookup(accessor, &self_)?.inner;
        let written = bucket.set(key, value, lifetime, when.map(Precondition::from)).await?;
        Ok(written.map(String::from).map_err(types::Conflict::from))
    }

    async fn set_many(
        accessor: &Accessor<T, Self>, self_: Resource<BucketHandle>,
        entries: Vec<(String, Vec<u8>)>, lifetime: types::Lifetime,
    ) -> Result<()> {
        let lifetime = Lifetime::try_from(lifetime)?;
        if let Some(key) = repeated(&entries) {
            return Err(Error::Invalid(format!("`{key}` is given more than once")));
        }
        let bucket = BucketHandle::lookup(accessor, &self_)?;
        for (key, value) in entries {
            let written = bucket.inner.set(key.clone(), value, lifetime, None).await?;
            written.map_err(|_found| bucket.conflicted(&key))?;
        }
        Ok(())
    }

    async fn delete(
        accessor: &Accessor<T, Self>, self_: Resource<BucketHandle>, key: String,
        when: Option<types::Revision>,
    ) -> Result<std::result::Result<(), types::Conflict>> {
        let bucket = BucketHandle::lookup(accessor, &self_)?.inner;
        let deleted = bucket.delete(key, when.map(Revision::from)).await?;
        Ok(deleted.map_err(types::Conflict::from))
    }

    async fn delete_many(
        accessor: &Accessor<T, Self>, self_: Resource<BucketHandle>, keys: Vec<String>,
    ) -> Result<()> {
        let bucket = BucketHandle::lookup(accessor, &self_)?;
        for key in keys {
            let deleted = bucket.inner.delete(key.clone(), None).await?;
            deleted.map_err(|_found| bucket.conflicted(&key))?;
        }
        Ok(())
    }

    async fn increment(
        accessor: &Accessor<T, Self>, self_: Resource<BucketHandle>, key: String, delta: i64,
        ttl_ms: Option<u64>,
    ) -> Result<i64> {
        let ttl = ttl_ms.map(ttl).transpose()?;
        let bucket = BucketHandle::lookup(accessor, &self_)?.inner;
        Ok(bucket.increment(key, delta, ttl).await?)
    }

    fn drop(mut host: Access<'_, T, Self>, rep: Resource<BucketHandle>) -> wasmtime::Result<()> {
        host.get().table.delete(rep)?;
        Ok(())
    }
}

impl TryFrom<types::Lifetime> for Lifetime {
    type Error = Error;

    fn try_from(lifetime: types::Lifetime) -> Result<Self> {
        Ok(match lifetime {
            types::Lifetime::Forever => Self::Replace(None),
            types::Lifetime::TtlMs(millis) => Self::Replace(Some(ttl(millis)?)),
            types::Lifetime::Keep => Self::Keep(None),
            types::Lifetime::KeepOrTtlMs(millis) => Self::Keep(Some(ttl(millis)?)),
        })
    }
}

// a zero TTL would expire the entry as it is written, so it writes nothing
fn ttl(millis: u64) -> Result<Duration> {
    match millis {
        0 => Err(Error::Invalid("a TTL must be greater than zero".to_owned())),
        millis => Ok(Duration::from_millis(millis)),
    }
}

impl From<types::Precondition> for Precondition {
    fn from(when: types::Precondition) -> Self {
        match when {
            types::Precondition::Absent => Self::Absent,
            types::Precondition::Matches(revision) => Self::Matches(revision.into()),
        }
    }
}

impl From<Entry> for types::Entry {
    fn from(entry: Entry) -> Self {
        Self {
            value: entry.value,
            revision: entry.revision.into(),
        }
    }
}

impl From<Option<Entry>> for types::Conflict {
    fn from(current: Option<Entry>) -> Self {
        Self {
            current: current.map(types::Entry::from),
        }
    }
}

fn repeated(entries: &[(String, Vec<u8>)]) -> Option<&str> {
    let mut seen = HashSet::new();
    entries.iter().map(|(key, _)| key.as_str()).find(|key| !seen.insert(*key))
}

impl Host for HostView<'_, dyn KeyValueClient> {}

impl HostBucket for HostView<'_, dyn KeyValueClient> {}
