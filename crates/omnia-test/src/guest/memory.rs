//! In-memory `StateStore` + `BlobStore` + `Vault` with real atomics and TTL.

use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use bytes::Bytes;
use omnia_sdk::capability::{self, ErrorKind};
use omnia_sdk::keyvalue::{Conflict, DEFAULT};
use omnia_sdk::{BlobStore, ContainerMetadata, ObjectMetadata, StateStore, Vault};

/// A state snapshot: bucket to key to raw bytes.
pub type StateSnapshot = BTreeMap<String, BTreeMap<String, Vec<u8>>>;
/// A blob snapshot: container to object name to bytes.
pub type BlobSnapshot = BTreeMap<String, BTreeMap<String, Vec<u8>>>;

/// In-memory state, blob, and secret storage; clones share one store.
///
/// Every method of all three traits is real: `compare_and_swap` and
/// `increment` hold the store's lock across their read-modify-write, and
/// `get_range` clamps an inclusive `end` the way the blobstore default does.
/// Reads on a container that was never created see it as empty; writes
/// create it. The derived `BlobStore` operations (`has`, `clear`,
/// `copy_object`, ...) compose these primitives.
///
/// State follows the [`StateStore`] contract against the real clock:
///
/// - A handle addresses the bucket [`DEFAULT`] until
///   [`named`](StateStore::named) returns one on another bucket of the same
///   store.
/// - [`bucket`](Self::bucket) declares a bucket, and a call on an undeclared
///   one fails with [`ErrorKind::NotFound`].
/// - A zero TTL fails with [`ErrorKind::Invalid`] and writes nothing.
///
/// ```
/// use omnia_sdk::{BlobStore as _, StateStore as _};
/// use omnia_test::guest::Memory;
///
/// # tokio::runtime::Runtime::new().unwrap().block_on(async {
/// let memory = Memory::default().bucket("sessions");
/// memory.named("sessions").set("ann", b"token", None).await.unwrap();
/// assert_eq!(memory.named("sessions").state("ann"), Some(b"token".to_vec()));
/// assert_eq!(memory.increment("visits", 5, None).await.unwrap(), 5);
/// memory.put("avatars", "ann.png", b"png").await.unwrap();
/// assert_eq!(memory.object("avatars", "ann.png"), Some(b"png".to_vec()));
/// # });
/// ```
#[derive(Clone, Debug)]
pub struct Memory {
    inner: Arc<Inner>,
    bucket: Arc<str>,
}

impl Default for Memory {
    fn default() -> Self {
        Self {
            inner: Arc::default(),
            bucket: Arc::from(DEFAULT),
        }
    }
}

impl Memory {
    /// Declares the bucket `name`, which every clone and named handle shares.
    #[must_use]
    pub fn bucket(self, name: &str) -> Self {
        self.lock_state().entry(name.to_owned()).or_default();
        self
    }

    fn lock_state(&self) -> MutexGuard<'_, BTreeMap<String, Bucket>> {
        let mut guard = self.inner.state.lock().expect("state lock");
        let now = Instant::now();
        for bucket in guard.values_mut() {
            bucket.retain(|_, entry| !entry.is_expired(now));
        }
        guard
    }

    fn lock_blobs(&self) -> MutexGuard<'_, BTreeMap<String, Container>> {
        self.inner.blobs.lock().expect("blob lock")
    }

    fn lock_secrets(&self) -> MutexGuard<'_, BTreeMap<String, Vec<u8>>> {
        self.inner.secrets.lock().expect("secrets lock")
    }

    fn tick(&self) -> u64 {
        self.inner.clock.fetch_add(1, Ordering::Relaxed) + 1
    }

    /// The state value at `key` in this handle's bucket.
    #[must_use]
    pub fn state(&self, key: &str) -> Option<Vec<u8>> {
        self.lock_state().get(&*self.bucket)?.get(key).map(|entry| entry.bytes.clone())
    }

    /// The secret value at `key`.
    #[must_use]
    pub fn secret(&self, key: &str) -> Option<Vec<u8>> {
        self.lock_secrets().get(key).cloned()
    }

    /// The stored bytes of `name` in `container`.
    #[must_use]
    pub fn object(&self, container: &str, name: &str) -> Option<Vec<u8>> {
        self.lock_blobs().get(container)?.objects.get(name).map(|object| object.data.clone())
    }

    /// The sorted object names in `container`.
    #[must_use]
    pub fn objects(&self, container: &str) -> Vec<String> {
        self.lock_blobs()
            .get(container)
            .map(|container| container.objects.keys().cloned().collect())
            .unwrap_or_default()
    }

    /// Seeds a state entry in this handle's bucket, with no TTL.
    ///
    /// # Panics
    ///
    /// Panics if the bucket is not declared.
    pub fn insert_state(&self, key: &str, bytes: &[u8]) {
        if let Err(err) = self.set_state(key, bytes, None) {
            panic!("seeding `{key}`: {err}");
        }
    }

    /// Seeds a secret entry.
    pub fn insert_secret(&self, key: &str, bytes: &[u8]) {
        drop(self.lock_secrets().insert(key.to_owned(), bytes.to_vec()));
    }

    /// Seeds an object, creating its container.
    pub fn insert_object(&self, container: &str, name: &str, bytes: &[u8]) {
        let created_at = self.tick();
        let mut blobs = self.lock_blobs();
        let container = blobs.entry(container.to_owned()).or_insert_with(|| Container {
            created_at,
            objects: BTreeMap::new(),
        });
        let replaced = container.objects.insert(
            name.to_owned(),
            Object {
                data: bytes.to_vec(),
                created_at,
            },
        );
        drop(blobs);
        drop(replaced);
    }

    /// Creates an empty container; idempotent.
    pub fn insert_container(&self, name: &str) {
        let created_at = self.tick();
        self.lock_blobs().entry(name.to_owned()).or_insert_with(|| Container {
            created_at,
            objects: BTreeMap::new(),
        });
    }

    /// Whether `name` was created or holds objects.
    #[must_use]
    pub fn has_container(&self, name: &str) -> bool {
        self.lock_blobs().contains_key(name)
    }

    /// Whether no state, secret, or object is stored.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.lock_state().values().all(BTreeMap::is_empty)
            && self.lock_secrets().is_empty()
            && self.lock_blobs().values().all(|c| c.objects.is_empty())
    }

    /// A snapshot of both stores for byte-stability comparisons.
    #[must_use]
    pub fn snapshot(&self) -> (StateSnapshot, BlobSnapshot) {
        let state = self
            .lock_state()
            .iter()
            .map(|(name, bucket)| {
                let entries =
                    bucket.iter().map(|(k, entry)| (k.clone(), entry.bytes.clone())).collect();
                (name.clone(), entries)
            })
            .collect();
        let blobs = self
            .lock_blobs()
            .iter()
            .map(|(name, container)| {
                let objects =
                    container.objects.iter().map(|(n, o)| (n.clone(), o.data.clone())).collect();
                (name.clone(), objects)
            })
            .collect();
        (state, blobs)
    }

    // -- state ------------------------------------------------------------

    fn in_bucket<T>(&self, f: impl FnOnce(&mut Bucket) -> T) -> capability::Result<T> {
        self.lock_state().get_mut(&*self.bucket).map(f).ok_or_else(|| {
            let message = format!("bucket `{}` is not declared", self.bucket);
            capability::Error::new(ErrorKind::NotFound, message)
        })
    }

    fn get_state(&self, key: &str) -> capability::Result<Option<Bytes>> {
        self.in_bucket(|bucket| bucket.get(key).map(|entry| Bytes::from(entry.bytes.clone())))
    }

    fn set_state(&self, key: &str, value: &[u8], ttl: Option<Duration>) -> capability::Result<()> {
        let ttl = checked(ttl)?;
        self.in_bucket(|bucket| {
            drop(bucket.insert(key.to_owned(), StateEntry::new(value.to_vec(), ttl)));
        })
    }

    fn delete_state(&self, key: &str) -> capability::Result<()> {
        self.in_bucket(|bucket| drop(bucket.remove(key)))
    }

    fn swap_state(
        &self, key: &str, current: Option<&[u8]>, new: &[u8], ttl: Option<Duration>,
    ) -> capability::Result<Result<(), Conflict>> {
        let ttl = checked(ttl)?;
        self.in_bucket(|bucket| {
            let existing = bucket.get(key).cloned();
            let observed = existing.as_ref().map(|entry| entry.bytes.as_slice());
            if observed != current {
                let current = observed.map(Bytes::copy_from_slice);
                return Err(Conflict { current });
            }
            drop(bucket.insert(key.to_owned(), StateEntry::atomic(existing, new.to_vec(), ttl)));
            Ok(())
        })
    }

    fn increment_state(
        &self, key: &str, delta: i64, ttl: Option<Duration>,
    ) -> capability::Result<i64> {
        let ttl = checked(ttl)?;
        self.in_bucket(|bucket| {
            let existing = bucket.get(key).cloned();
            let incremented = add_i64(existing.as_ref().map(|entry| entry.bytes.as_slice()), delta)
                .with_context(|| format!("incrementing `{key}` by {delta}"))
                .map_err(|err| capability::Error::new(ErrorKind::Invalid, format!("{err:#}")))?;
            let entry = StateEntry::atomic(existing, incremented.to_be_bytes().to_vec(), ttl);
            drop(bucket.insert(key.to_owned(), entry));
            Ok(incremented)
        })?
    }

    // -- blobs ------------------------------------------------------------

    fn delete_object(&self, container: &str, name: &str) {
        if let Some(container) = self.lock_blobs().get_mut(container) {
            drop(container.objects.remove(name));
        }
    }

    fn delete_container(&self, name: &str) {
        drop(self.lock_blobs().remove(name));
    }

    fn range(&self, container: &str, name: &str, start: u64, end: u64) -> Result<Vec<u8>> {
        let data = self
            .object(container, name)
            .ok_or_else(|| anyhow!("object not found: {container}/{name}"))?;
        // as the blobstore default: `end` of 0 or `u64::MAX` reads to the end, else inclusive
        let unbounded = end == 0 || end == u64::MAX;
        if !unbounded && end < start {
            bail!("invalid byte range: end ({end}) < start ({start})");
        }
        let len = data.len() as u64;
        let from = start.min(len);
        let to = if unbounded { len } else { end.saturating_add(1).min(len) };
        let from = usize::try_from(from).expect("clamped to the object's length");
        let to = usize::try_from(to).expect("clamped to the object's length");
        Ok(data[from..to].to_vec())
    }

    fn object_info(&self, container: &str, name: &str) -> Result<ObjectMetadata> {
        let blobs = self.lock_blobs();
        let info =
            blobs.get(container).and_then(|c| c.objects.get(name)).map(|object| ObjectMetadata {
                name: name.to_owned(),
                container: container.to_owned(),
                created_at: object.created_at,
                size: object.data.len() as u64,
            });
        drop(blobs);
        info.ok_or_else(|| anyhow!("object not found: {container}/{name}"))
    }

    fn container_info(&self, name: &str) -> Result<ContainerMetadata> {
        let blobs = self.lock_blobs();
        let info = blobs.get(name).map(|container| ContainerMetadata {
            name: name.to_owned(),
            created_at: container.created_at,
        });
        drop(blobs);
        info.ok_or_else(|| anyhow!("container not found: {name}"))
    }
}

// a zero TTL would expire the entry as it is written, so it writes nothing
fn checked(ttl: Option<Duration>) -> capability::Result<Option<Duration>> {
    if ttl == Some(Duration::ZERO) {
        return Err(capability::Error::new(ErrorKind::Invalid, "a TTL must be greater than zero"));
    }
    Ok(ttl)
}

impl StateStore for Memory {
    type Named = Self;

    fn get(&self, key: &str) -> impl Future<Output = capability::Result<Option<Bytes>>> + Send {
        ready(self.get_state(key))
    }

    fn set(
        &self, key: &str, value: &[u8], ttl: Option<Duration>,
    ) -> impl Future<Output = capability::Result<()>> + Send {
        ready(self.set_state(key, value, ttl))
    }

    fn delete(&self, key: &str) -> impl Future<Output = capability::Result<()>> + Send {
        ready(self.delete_state(key))
    }

    fn compare_and_swap(
        &self, key: &str, current: Option<&[u8]>, new: &[u8], ttl: Option<Duration>,
    ) -> impl Future<Output = capability::Result<Result<(), Conflict>>> + Send {
        ready(self.swap_state(key, current, new, ttl))
    }

    fn increment(
        &self, key: &str, delta: i64, ttl: Option<Duration>,
    ) -> impl Future<Output = capability::Result<i64>> + Send {
        ready(self.increment_state(key, delta, ttl))
    }

    fn named(&self, name: &str) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
            bucket: Arc::from(name),
        }
    }
}

impl BlobStore for Memory {
    fn get(
        &self, container: &str, name: &str,
    ) -> impl Future<Output = Result<Option<Vec<u8>>>> + Send {
        ready(Ok(self.object(container, name)))
    }

    fn put(
        &self, container: &str, name: &str, data: &[u8],
    ) -> impl Future<Output = Result<()>> + Send {
        self.insert_object(container, name, data);
        ready(Ok(()))
    }

    fn delete(&self, container: &str, name: &str) -> impl Future<Output = Result<()>> + Send {
        self.delete_object(container, name);
        ready(Ok(()))
    }

    fn list(&self, container: &str) -> impl Future<Output = Result<Vec<String>>> + Send {
        ready(Ok(self.objects(container)))
    }

    fn get_range(
        &self, container: &str, name: &str, start: u64, end: u64,
    ) -> impl Future<Output = Result<Vec<u8>>> + Send {
        ready(self.range(container, name, start, end))
    }

    fn object_info(
        &self, container: &str, name: &str,
    ) -> impl Future<Output = Result<ObjectMetadata>> + Send {
        ready(Self::object_info(self, container, name))
    }

    fn create_container(&self, name: &str) -> impl Future<Output = Result<()>> + Send {
        self.insert_container(name);
        ready(Ok(()))
    }

    fn delete_container(&self, name: &str) -> impl Future<Output = Result<()>> + Send {
        Self::delete_container(self, name);
        ready(Ok(()))
    }

    fn container_exists(&self, name: &str) -> impl Future<Output = Result<bool>> + Send {
        ready(Ok(self.has_container(name)))
    }

    fn container_info(
        &self, container: &str,
    ) -> impl Future<Output = Result<ContainerMetadata>> + Send {
        ready(Self::container_info(self, container))
    }
}

impl Vault for Memory {
    fn get(&self, key: &str) -> impl Future<Output = Result<Option<Vec<u8>>>> + Send {
        ready(Ok(self.secret(key)))
    }

    fn set(&self, key: &str, value: &[u8]) -> impl Future<Output = Result<()>> + Send {
        self.insert_secret(key, value);
        ready(Ok(()))
    }

    fn delete(&self, key: &str) -> impl Future<Output = Result<()>> + Send {
        drop(self.lock_secrets().remove(key));
        ready(Ok(()))
    }

    fn exists(&self, key: &str) -> impl Future<Output = Result<bool>> + Send {
        ready(Ok(self.lock_secrets().contains_key(key)))
    }

    fn list_ids(&self) -> impl Future<Output = Result<Vec<String>>> + Send {
        ready(Ok(self.lock_secrets().keys().cloned().collect()))
    }
}

#[derive(Debug)]
struct Inner {
    // a bucket is declared once it has a map, even an empty one
    state: Mutex<BTreeMap<String, Bucket>>,
    blobs: Mutex<BTreeMap<String, Container>>,
    secrets: Mutex<BTreeMap<String, Vec<u8>>>,
    // Metadata timestamps come from this counter, not the wall clock, so
    // `*_info` reads are deterministic and ordered by creation.
    clock: AtomicU64,
}

impl Default for Inner {
    fn default() -> Self {
        Self {
            state: Mutex::new(BTreeMap::from([(DEFAULT.to_owned(), Bucket::new())])),
            blobs: Mutex::default(),
            secrets: Mutex::default(),
            clock: AtomicU64::default(),
        }
    }
}

type Bucket = BTreeMap<String, StateEntry>;

#[derive(Clone, Debug)]
struct StateEntry {
    bytes: Vec<u8>,
    // `None` never expires; a TTL too long for an `Instant` is the same
    expires_at: Option<Instant>,
}

impl StateEntry {
    fn new(bytes: Vec<u8>, ttl: Option<Duration>) -> Self {
        Self {
            bytes,
            expires_at: ttl.and_then(|ttl| Instant::now().checked_add(ttl)),
        }
    }

    // an atomic applies its TTL only on create; a live entry keeps the time it has left
    fn atomic(existing: Option<Self>, bytes: Vec<u8>, ttl: Option<Duration>) -> Self {
        match existing {
            Some(existing) => Self { bytes, ..existing },
            None => Self::new(bytes, ttl),
        }
    }

    fn is_expired(&self, now: Instant) -> bool {
        self.expires_at.is_some_and(|at| now >= at)
    }
}

#[derive(Clone, Debug, Default)]
struct Container {
    created_at: u64,
    objects: BTreeMap<String, Object>,
}

#[derive(Clone, Debug)]
struct Object {
    data: Vec<u8>,
    created_at: u64,
}

fn add_i64(current: Option<&[u8]>, delta: i64) -> Result<i64> {
    let base = match current {
        None => 0,
        Some(value) => {
            let bytes: [u8; 8] = value.try_into().map_err(|_len| {
                anyhow!("value is {} bytes, not an 8-byte big-endian integer", value.len())
            })?;
            i64::from_be_bytes(bytes)
        }
    };
    base.checked_add(delta).context("adding delta overflows i64")
}

/// A prefixed view over one shared [`Memory`]: every key, secret, and
/// container is scoped under a prefix, modelling a tenant- or project-keyed
/// host binding.
///
/// [`named`](StateStore::named) keeps the prefix over the bucket it selects.
///
/// ```
/// use omnia_sdk::StateStore as _;
/// use omnia_test::guest::{Memory, Namespaced};
///
/// # tokio::runtime::Runtime::new().unwrap().block_on(async {
/// let memory = Memory::default();
/// let tenant = Namespaced::new("acme", memory.clone());
/// tenant.set("plan", b"pro", None).await.unwrap();
/// assert_eq!(memory.state("acme/plan"), Some(b"pro".to_vec()));
/// assert_eq!(memory.state("plan"), None);
/// # });
/// ```
#[derive(Clone, Debug)]
pub struct Namespaced {
    prefix: String,
    inner: Memory,
}

impl Namespaced {
    /// Scopes `inner` under `prefix` (`Memory` is already a shared handle, so
    /// the caller keeps its own clone as the probe).
    pub fn new(prefix: impl Into<String>, inner: Memory) -> Self {
        Self {
            prefix: prefix.into(),
            inner,
        }
    }

    /// The underlying store.
    #[must_use]
    pub const fn memory(&self) -> &Memory {
        &self.inner
    }

    fn scoped(&self, name: &str) -> String {
        format!("{}/{name}", self.prefix)
    }

    fn unscoped(&self, name: &str) -> String {
        name.strip_prefix(&format!("{}/", self.prefix)).unwrap_or(name).to_owned()
    }
}

impl StateStore for Namespaced {
    type Named = Self;

    fn get(&self, key: &str) -> impl Future<Output = capability::Result<Option<Bytes>>> + Send {
        ready(self.inner.get_state(&self.scoped(key)))
    }

    fn set(
        &self, key: &str, value: &[u8], ttl: Option<Duration>,
    ) -> impl Future<Output = capability::Result<()>> + Send {
        ready(self.inner.set_state(&self.scoped(key), value, ttl))
    }

    fn delete(&self, key: &str) -> impl Future<Output = capability::Result<()>> + Send {
        ready(self.inner.delete_state(&self.scoped(key)))
    }

    fn compare_and_swap(
        &self, key: &str, current: Option<&[u8]>, new: &[u8], ttl: Option<Duration>,
    ) -> impl Future<Output = capability::Result<Result<(), Conflict>>> + Send {
        ready(self.inner.swap_state(&self.scoped(key), current, new, ttl))
    }

    fn increment(
        &self, key: &str, delta: i64, ttl: Option<Duration>,
    ) -> impl Future<Output = capability::Result<i64>> + Send {
        ready(self.inner.increment_state(&self.scoped(key), delta, ttl))
    }

    fn named(&self, name: &str) -> Self {
        Self {
            prefix: self.prefix.clone(),
            inner: self.inner.named(name),
        }
    }
}

impl BlobStore for Namespaced {
    fn get(
        &self, container: &str, name: &str,
    ) -> impl Future<Output = Result<Option<Vec<u8>>>> + Send {
        ready(Ok(self.inner.object(&self.scoped(container), name)))
    }

    fn put(
        &self, container: &str, name: &str, data: &[u8],
    ) -> impl Future<Output = Result<()>> + Send {
        self.inner.insert_object(&self.scoped(container), name, data);
        ready(Ok(()))
    }

    fn delete(&self, container: &str, name: &str) -> impl Future<Output = Result<()>> + Send {
        self.inner.delete_object(&self.scoped(container), name);
        ready(Ok(()))
    }

    fn list(&self, container: &str) -> impl Future<Output = Result<Vec<String>>> + Send {
        ready(Ok(self.inner.objects(&self.scoped(container))))
    }

    fn get_range(
        &self, container: &str, name: &str, start: u64, end: u64,
    ) -> impl Future<Output = Result<Vec<u8>>> + Send {
        ready(self.inner.range(&self.scoped(container), name, start, end))
    }

    fn object_info(
        &self, container: &str, name: &str,
    ) -> impl Future<Output = Result<ObjectMetadata>> + Send {
        let info =
            self.inner.object_info(&self.scoped(container), name).map(|info| ObjectMetadata {
                container: self.unscoped(&info.container),
                ..info
            });
        ready(info)
    }

    fn create_container(&self, name: &str) -> impl Future<Output = Result<()>> + Send {
        self.inner.insert_container(&self.scoped(name));
        ready(Ok(()))
    }

    fn delete_container(&self, name: &str) -> impl Future<Output = Result<()>> + Send {
        self.inner.delete_container(&self.scoped(name));
        ready(Ok(()))
    }

    fn container_exists(&self, name: &str) -> impl Future<Output = Result<bool>> + Send {
        ready(Ok(self.inner.has_container(&self.scoped(name))))
    }

    fn container_info(
        &self, container: &str,
    ) -> impl Future<Output = Result<ContainerMetadata>> + Send {
        let info =
            self.inner.container_info(&self.scoped(container)).map(|info| ContainerMetadata {
                name: self.unscoped(&info.name),
                ..info
            });
        ready(info)
    }
}

impl Vault for Namespaced {
    fn get(&self, key: &str) -> impl Future<Output = Result<Option<Vec<u8>>>> + Send {
        ready(Ok(self.inner.secret(&self.scoped(key))))
    }

    fn set(&self, key: &str, value: &[u8]) -> impl Future<Output = Result<()>> + Send {
        self.inner.insert_secret(&self.scoped(key), value);
        ready(Ok(()))
    }

    fn delete(&self, key: &str) -> impl Future<Output = Result<()>> + Send {
        drop(self.inner.lock_secrets().remove(&self.scoped(key)));
        ready(Ok(()))
    }

    fn exists(&self, key: &str) -> impl Future<Output = Result<bool>> + Send {
        ready(Ok(self.inner.lock_secrets().contains_key(&self.scoped(key))))
    }

    fn list_ids(&self) -> impl Future<Output = Result<Vec<String>>> + Send {
        let prefix = format!("{}/", self.prefix);
        let ids: Vec<String> = self
            .inner
            .lock_secrets()
            .keys()
            .filter_map(|key| key.strip_prefix(&prefix).map(str::to_owned))
            .collect();
        ready(Ok(ids))
    }
}
