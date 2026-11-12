//! In-memory `StateStore` + `BlobStore` + `SecretStore` with real atomics and TTL.

use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::ops::Range;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow};
use bytes::Bytes;
use omnia_sdk::blobstore::{ContainerMetadata, ObjectMetadata};
use omnia_sdk::capability::{self, ErrorKind, Page};
use omnia_sdk::keyvalue::{Conflict, DEFAULT};
use omnia_sdk::{BlobStore, SecretStore, StateStore};

/// A state snapshot: bucket to key to raw bytes.
pub type StateSnapshot = BTreeMap<String, BTreeMap<String, Vec<u8>>>;
/// A blob snapshot: container to object name to bytes.
pub type BlobSnapshot = BTreeMap<String, BTreeMap<String, Vec<u8>>>;

/// In-memory state, blob, and secret storage; clones share one store.
///
/// Every method of all three traits is real: `compare_and_swap` and
/// `increment` hold the store's lock across their read-modify-write, and
/// `get_range` clamps its half-open range to the object. Reads on a
/// container that was never created see it as empty; writes create it. The
/// provided `BlobStore` operations (`exists`, `clear`, `copy`, ...) compose
/// these primitives. A listing answers one page, and resumes after the key
/// or name it is given.
///
/// A handle addresses one state bucket and one secret locker, both named
/// [`DEFAULT`] until [`named`](Self::named) returns a handle on another name
/// of the same store. Lockers open on first use. State follows the
/// [`StateStore`] contract against the real clock:
///
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
    name: Arc<str>,
}

impl Default for Memory {
    fn default() -> Self {
        Self {
            inner: Arc::default(),
            name: Arc::from(DEFAULT),
        }
    }
}

impl Memory {
    /// A handle on the bucket and locker `name` of the same store.
    ///
    /// [`StateStore::named`] and [`SecretStore::named`] both return it.
    #[must_use]
    pub fn named(&self, name: &str) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
            name: Arc::from(name),
        }
    }

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

    fn lock_secrets(&self) -> MutexGuard<'_, BTreeMap<String, Locker>> {
        self.inner.secrets.lock().expect("secrets lock")
    }

    fn tick(&self) -> u64 {
        self.inner.clock.fetch_add(1, Ordering::Relaxed) + 1
    }

    /// The state value at `key` in this handle's bucket.
    #[must_use]
    pub fn state(&self, key: &str) -> Option<Vec<u8>> {
        self.lock_state().get(&*self.name)?.get(key).map(|entry| entry.bytes.clone())
    }

    /// The secret value at `key` in this handle's locker.
    #[must_use]
    pub fn secret(&self, key: &str) -> Option<Vec<u8>> {
        self.lock_secrets().get(&*self.name)?.get(key).cloned()
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

    /// Seeds a secret entry in this handle's locker.
    pub fn insert_secret(&self, key: &str, bytes: &[u8]) {
        let mut secrets = self.lock_secrets();
        let locker = secrets.entry(self.name.to_string()).or_default();
        drop(locker.insert(key.to_owned(), bytes.to_vec()));
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
            && self.lock_secrets().values().all(BTreeMap::is_empty)
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
        self.lock_state().get_mut(&*self.name).map(f).ok_or_else(|| {
            let message = format!("bucket `{}` is not declared", self.name);
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

    fn range(&self, container: &str, name: &str, range: Range<u64>) -> Option<Bytes> {
        let data = self.object(container, name)?;
        let len = data.len() as u64;
        let end = range.end.min(len);
        let start = range.start.min(end);
        let start = usize::try_from(start).expect("clamped to the object's length");
        let end = usize::try_from(end).expect("clamped to the object's length");
        Some(Bytes::copy_from_slice(&data[start..end]))
    }

    fn object_info(&self, container: &str, name: &str) -> Option<ObjectMetadata> {
        let blobs = self.lock_blobs();
        let object = blobs.get(container)?.objects.get(name)?;
        Some(ObjectMetadata::new(
            name,
            container,
            timestamp(object.created_at),
            object.data.len() as u64,
        ))
    }

    fn listing(&self, container: &str, prefix: &str, after: Option<&str>) -> Vec<ObjectMetadata> {
        let blobs = self.lock_blobs();
        let Some(objects) = blobs.get(container).map(|container| &container.objects) else {
            return Vec::new();
        };
        objects
            .iter()
            .filter(|(name, _)| {
                name.starts_with(prefix) && after.is_none_or(|after| name.as_str() > after)
            })
            .map(|(name, object)| {
                let size = object.data.len() as u64;
                ObjectMetadata::new(name, container, timestamp(object.created_at), size)
            })
            .collect()
    }

    fn container_info(&self, name: &str) -> Option<ContainerMetadata> {
        let blobs = self.lock_blobs();
        let container = blobs.get(name)?;
        Some(ContainerMetadata::new(name, timestamp(container.created_at)))
    }

    // -- secrets ----------------------------------------------------------

    fn delete_secret(&self, key: &str) {
        if let Some(locker) = self.lock_secrets().get_mut(&*self.name) {
            drop(locker.remove(key));
        }
    }

    fn secret_keys(&self, after: Option<&str>) -> Vec<String> {
        self.lock_secrets()
            .get(&*self.name)
            .map(|locker| {
                locker
                    .keys()
                    .filter(|key| after.is_none_or(|after| key.as_str() > after))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }
}

// a zero TTL would expire the entry as it is written, so it writes nothing
fn checked(ttl: Option<Duration>) -> capability::Result<Option<Duration>> {
    if ttl == Some(Duration::ZERO) {
        return Err(capability::Error::new(ErrorKind::Invalid, "a TTL must be greater than zero"));
    }
    Ok(ttl)
}

// metadata timestamps count ticks of the store's clock from the epoch, so
// they are deterministic and ordered by creation
fn timestamp(tick: u64) -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(tick)
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
        Self::named(self, name)
    }
}

impl BlobStore for Memory {
    fn get(
        &self, container: &str, name: &str,
    ) -> impl Future<Output = capability::Result<Option<Bytes>>> + Send {
        ready(Ok(self.object(container, name).map(Bytes::from)))
    }

    fn get_range(
        &self, container: &str, name: &str, range: Range<u64>,
    ) -> impl Future<Output = capability::Result<Option<Bytes>>> + Send {
        ready(Ok(self.range(container, name, range)))
    }

    fn put(
        &self, container: &str, name: &str, data: &[u8],
    ) -> impl Future<Output = capability::Result<()>> + Send {
        self.insert_object(container, name, data);
        ready(Ok(()))
    }

    fn delete(
        &self, container: &str, name: &str,
    ) -> impl Future<Output = capability::Result<()>> + Send {
        self.delete_object(container, name);
        ready(Ok(()))
    }

    fn stat(
        &self, container: &str, name: &str,
    ) -> impl Future<Output = capability::Result<Option<ObjectMetadata>>> + Send {
        ready(Ok(self.object_info(container, name)))
    }

    fn list(
        &self, container: &str, prefix: &str, after: Option<&str>,
    ) -> impl Future<Output = capability::Result<Page<ObjectMetadata>>> + Send {
        ready(Ok(Page::new(self.listing(container, prefix, after), None)))
    }

    fn create_container(
        &self, container: &str,
    ) -> impl Future<Output = capability::Result<()>> + Send {
        self.insert_container(container);
        ready(Ok(()))
    }

    fn delete_container(
        &self, container: &str,
    ) -> impl Future<Output = capability::Result<()>> + Send {
        Self::delete_container(self, container);
        ready(Ok(()))
    }

    fn container_exists(
        &self, container: &str,
    ) -> impl Future<Output = capability::Result<bool>> + Send {
        ready(Ok(self.has_container(container)))
    }

    fn container_info(
        &self, container: &str,
    ) -> impl Future<Output = capability::Result<Option<ContainerMetadata>>> + Send {
        ready(Ok(Self::container_info(self, container)))
    }
}

impl SecretStore for Memory {
    type Named = Self;

    fn get(&self, key: &str) -> impl Future<Output = capability::Result<Option<Bytes>>> + Send {
        ready(Ok(self.secret(key).map(Bytes::from)))
    }

    fn set(&self, key: &str, value: &[u8]) -> impl Future<Output = capability::Result<()>> + Send {
        self.insert_secret(key, value);
        ready(Ok(()))
    }

    fn delete(&self, key: &str) -> impl Future<Output = capability::Result<()>> + Send {
        self.delete_secret(key);
        ready(Ok(()))
    }

    fn keys(
        &self, after: Option<&str>,
    ) -> impl Future<Output = capability::Result<Page<String>>> + Send {
        ready(Ok(Page::new(self.secret_keys(after), None)))
    }

    fn named(&self, name: &str) -> Self {
        Self::named(self, name)
    }
}

#[derive(Debug)]
struct Inner {
    // a bucket is declared once it has a map, even an empty one
    state: Mutex<BTreeMap<String, Bucket>>,
    blobs: Mutex<BTreeMap<String, Container>>,
    secrets: Mutex<BTreeMap<String, Locker>>,
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

type Locker = BTreeMap<String, Vec<u8>>;

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
