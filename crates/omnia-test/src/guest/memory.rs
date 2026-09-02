//! In-memory `StateStore` + `BlobStore` with real atomics.

use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use anyhow::{Context, Result, anyhow, bail};
use omnia_guest::{BlobStore, CasError, ContainerMetadata, ObjectMetadata, StateStore};

/// A state snapshot: key to raw bytes.
pub type State = BTreeMap<String, Vec<u8>>;
/// A blob snapshot: container to object name to bytes.
pub type Blobs = BTreeMap<String, BTreeMap<String, Vec<u8>>>;

#[derive(Clone, Debug)]
struct Object {
    data: Vec<u8>,
    created_at: u64,
}

#[derive(Clone, Debug, Default)]
struct Container {
    created_at: u64,
    objects: BTreeMap<String, Object>,
}

#[derive(Debug, Default)]
struct Inner {
    state: Mutex<State>,
    blobs: Mutex<BTreeMap<String, Container>>,
    // Metadata timestamps come from this counter, not the wall clock, so
    // `*_info` reads are deterministic and ordered by creation.
    clock: AtomicU64,
}

/// In-memory state and blob storage; clones share one store.
///
/// Every method of both traits is real: `cas` and `increment` hold the
/// store's lock across their read-modify-write, `increment` uses the
/// 8-byte big-endian encoding of the runtime's in-memory key-value default,
/// and `get_range` clamps an inclusive `end` the way the blobstore default
/// does. Reads on a container that was never created see it as empty;
/// writes create it.
///
/// ```
/// use omnia_guest::{BlobStore as _, StateStore as _};
/// use omnia_test::guest::Memory;
///
/// # tokio::runtime::Runtime::new().unwrap().block_on(async {
/// let memory = Memory::default();
/// memory.insert_state("visits", &5_i64.to_be_bytes());
/// assert_eq!(memory.increment("visits", 1).await.unwrap(), 6);
/// memory.put("avatars", "ann.png", b"png").await.unwrap();
/// assert_eq!(memory.object("avatars", "ann.png"), Some(b"png".to_vec()));
/// # });
/// ```
#[derive(Clone, Debug, Default)]
pub struct Memory {
    inner: Arc<Inner>,
}

impl Memory {
    fn lock_state(&self) -> MutexGuard<'_, State> {
        self.inner.state.lock().expect("state lock")
    }

    fn lock_blobs(&self) -> MutexGuard<'_, BTreeMap<String, Container>> {
        self.inner.blobs.lock().expect("blob lock")
    }

    fn tick(&self) -> u64 {
        self.inner.clock.fetch_add(1, Ordering::Relaxed) + 1
    }

    /// The state value at `key`.
    #[must_use]
    pub fn state(&self, key: &str) -> Option<Vec<u8>> {
        self.lock_state().get(key).cloned()
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

    /// Seeds a state entry.
    pub fn insert_state(&self, key: &str, bytes: &[u8]) {
        drop(self.lock_state().insert(key.to_owned(), bytes.to_vec()));
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

    /// Whether neither state nor any object is stored.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.lock_state().is_empty() && self.lock_blobs().values().all(|c| c.objects.is_empty())
    }

    /// A snapshot of both stores for byte-stability comparisons.
    #[must_use]
    pub fn snapshot(&self) -> (State, Blobs) {
        let blobs = self
            .lock_blobs()
            .iter()
            .map(|(name, container)| {
                let objects =
                    container.objects.iter().map(|(n, o)| (n.clone(), o.data.clone())).collect();
                (name.clone(), objects)
            })
            .collect();
        (self.lock_state().clone(), blobs)
    }

    // -- state ------------------------------------------------------------

    fn set_state(&self, key: &str, value: &[u8]) -> Option<Vec<u8>> {
        self.lock_state().insert(key.to_owned(), value.to_vec())
    }

    fn delete_state(&self, key: &str) {
        drop(self.lock_state().remove(key));
    }

    fn cas_state(&self, key: &str, expected: Option<&[u8]>, value: &[u8]) -> Result<(), CasError> {
        let mut state = self.lock_state();
        let observed = state.get(key).cloned();
        let swapped = if observed.as_deref() == expected {
            drop(state.insert(key.to_owned(), value.to_vec()));
            Ok(())
        } else {
            Err(CasError::Conflict(observed))
        };
        drop(state);
        swapped
    }

    fn increment_state(&self, key: &str, delta: i64) -> Result<i64> {
        let mut state = self.lock_state();
        let incremented = add_i64(state.get(key).map(Vec::as_slice), delta)
            .with_context(|| format!("incrementing `{key}` by {delta}"));
        if let Ok(incremented) = incremented {
            drop(state.insert(key.to_owned(), incremented.to_be_bytes().to_vec()));
        }
        drop(state);
        incremented
    }

    // -- blobs ------------------------------------------------------------

    fn delete_object(&self, container: &str, name: &str) {
        if let Some(container) = self.lock_blobs().get_mut(container) {
            drop(container.objects.remove(name));
        }
    }

    fn delete_objects(&self, container: &str, names: &[String]) {
        if let Some(container) = self.lock_blobs().get_mut(container) {
            for name in names {
                drop(container.objects.remove(name));
            }
        }
    }

    fn clear(&self, container: &str) {
        if let Some(container) = self.lock_blobs().get_mut(container) {
            container.objects.clear();
        }
    }

    fn delete_container(&self, name: &str) {
        drop(self.lock_blobs().remove(name));
    }

    fn range(&self, container: &str, name: &str, start: u64, end: u64) -> Result<Vec<u8>> {
        let data = self
            .object(container, name)
            .ok_or_else(|| anyhow!("object not found: {container}/{name}"))?;
        // Range semantics match the blobstore default: `end` of 0 or
        // `u64::MAX` reads to the end; otherwise `end` is inclusive, clamped
        // to the object's length.
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

    fn copy(
        &self, src_container: &str, src_name: &str, dest_container: &str, dest_name: &str,
    ) -> Result<()> {
        let data = self
            .object(src_container, src_name)
            .ok_or_else(|| anyhow!("object not found: {src_container}/{src_name}"))?;
        self.insert_object(dest_container, dest_name, &data);
        Ok(())
    }

    fn rename(
        &self, src_container: &str, src_name: &str, dest_container: &str, dest_name: &str,
    ) -> Result<()> {
        self.copy(src_container, src_name, dest_container, dest_name)?;
        self.delete_object(src_container, src_name);
        Ok(())
    }
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

impl StateStore for Memory {
    fn get(&self, key: &str) -> impl Future<Output = Result<Option<Vec<u8>>>> + Send {
        ready(Ok(self.state(key)))
    }

    fn set(
        &self, key: &str, value: &[u8], _ttl_secs: Option<u64>,
    ) -> impl Future<Output = Result<Option<Vec<u8>>>> + Send {
        ready(Ok(self.set_state(key, value)))
    }

    fn delete(&self, key: &str) -> impl Future<Output = Result<()>> + Send {
        self.delete_state(key);
        ready(Ok(()))
    }

    fn cas(
        &self, key: &str, expected: Option<&[u8]>, value: &[u8],
    ) -> impl Future<Output = Result<(), CasError>> + Send {
        ready(self.cas_state(key, expected, value))
    }

    fn increment(&self, key: &str, delta: i64) -> impl Future<Output = Result<i64>> + Send {
        ready(self.increment_state(key, delta))
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

    fn has(&self, container: &str, name: &str) -> impl Future<Output = Result<bool>> + Send {
        ready(Ok(self.object(container, name).is_some()))
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

    fn delete_objects(
        &self, container: &str, names: &[String],
    ) -> impl Future<Output = Result<()>> + Send {
        Self::delete_objects(self, container, names);
        ready(Ok(()))
    }

    fn clear(&self, container: &str) -> impl Future<Output = Result<()>> + Send {
        Self::clear(self, container);
        ready(Ok(()))
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

    fn copy_object(
        &self, src_container: &str, src_name: &str, dest_container: &str, dest_name: &str,
    ) -> impl Future<Output = Result<()>> + Send {
        ready(self.copy(src_container, src_name, dest_container, dest_name))
    }

    fn move_object(
        &self, src_container: &str, src_name: &str, dest_container: &str, dest_name: &str,
    ) -> impl Future<Output = Result<()>> + Send {
        ready(self.rename(src_container, src_name, dest_container, dest_name))
    }
}

/// A prefixed view over one shared [`Memory`]: every key and container is
/// scoped under a prefix, modelling a tenant- or project-keyed host binding.
///
/// ```
/// use omnia_guest::StateStore as _;
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
    fn get(&self, key: &str) -> impl Future<Output = Result<Option<Vec<u8>>>> + Send {
        ready(Ok(self.inner.state(&self.scoped(key))))
    }

    fn set(
        &self, key: &str, value: &[u8], _ttl_secs: Option<u64>,
    ) -> impl Future<Output = Result<Option<Vec<u8>>>> + Send {
        ready(Ok(self.inner.set_state(&self.scoped(key), value)))
    }

    fn delete(&self, key: &str) -> impl Future<Output = Result<()>> + Send {
        self.inner.delete_state(&self.scoped(key));
        ready(Ok(()))
    }

    fn cas(
        &self, key: &str, expected: Option<&[u8]>, value: &[u8],
    ) -> impl Future<Output = Result<(), CasError>> + Send {
        ready(self.inner.cas_state(&self.scoped(key), expected, value))
    }

    fn increment(&self, key: &str, delta: i64) -> impl Future<Output = Result<i64>> + Send {
        ready(self.inner.increment_state(&self.scoped(key), delta))
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

    fn has(&self, container: &str, name: &str) -> impl Future<Output = Result<bool>> + Send {
        ready(Ok(self.inner.object(&self.scoped(container), name).is_some()))
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

    fn delete_objects(
        &self, container: &str, names: &[String],
    ) -> impl Future<Output = Result<()>> + Send {
        self.inner.delete_objects(&self.scoped(container), names);
        ready(Ok(()))
    }

    fn clear(&self, container: &str) -> impl Future<Output = Result<()>> + Send {
        self.inner.clear(&self.scoped(container));
        ready(Ok(()))
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

    fn copy_object(
        &self, src_container: &str, src_name: &str, dest_container: &str, dest_name: &str,
    ) -> impl Future<Output = Result<()>> + Send {
        ready(self.inner.copy(
            &self.scoped(src_container),
            src_name,
            &self.scoped(dest_container),
            dest_name,
        ))
    }

    fn move_object(
        &self, src_container: &str, src_name: &str, dest_container: &str, dest_name: &str,
    ) -> impl Future<Output = Result<()>> + Send {
        ready(self.inner.rename(
            &self.scoped(src_container),
            src_name,
            &self.scoped(dest_container),
            dest_name,
        ))
    }
}
