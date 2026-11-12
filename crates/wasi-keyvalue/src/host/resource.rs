use std::fmt::Debug;
use std::sync::Arc;
use std::time::Duration;

pub use omnia_core::FutureResult;

/// Providers implement the [`Bucket`] trait to allow the host to
/// interact with different backend buckets (stores).
///
/// A TTL is a lifetime the backend enforces:
///
/// - An entry written without one never expires.
/// - A TTL rounds up to the store's resolution, so an entry never expires
///   early.
/// - [`set`](Self::set) replaces an entry and its lifetime.
/// - [`compare_and_swap`](Self::compare_and_swap) and
///   [`increment`](Self::increment) apply their TTL only when they create the
///   entry. An existing entry keeps the time it has left.
///
/// A backend stores every portable key, and fails a key it cannot store with
/// [`Error::Invalid`](crate::Error::Invalid). A portable key:
///
/// - is not empty;
/// - holds only ASCII letters, digits, `.`, `_`, and `-`;
/// - neither starts nor ends with `.`.
///
/// The host rejects a zero TTL before a backend sees it. A backend picks the
/// kind of a failure by returning an [`Error`](crate::Error), and any other
/// error reaches the guest as [`Error::Other`](crate::Error::Other).
pub trait Bucket: Debug + Send + Sync + 'static {
    /// Get the value associated with the key.
    fn get(&self, key: String) -> FutureResult<Option<Vec<u8>>>;

    /// Set the value associated with the key, replacing its lifetime.
    fn set(&self, key: String, value: Vec<u8>, ttl: Option<Duration>) -> FutureResult<()>;

    /// Delete the value associated with the key.
    fn delete(&self, key: String) -> FutureResult<()>;

    /// Check if the entry exists.
    fn exists(&self, key: String) -> FutureResult<bool>;

    /// List all keys in the bucket.
    fn keys(&self) -> FutureResult<Vec<String>>;

    /// Atomically add `delta` to the integer at `key`, starting from zero,
    /// and return the new value.
    fn increment(&self, key: String, delta: i64, ttl: Option<Duration>) -> FutureResult<i64>;

    /// Atomically replace the value at `key` with `new` while it still
    /// equals `current` (`None` means absent).
    ///
    /// A mismatch changes nothing and returns the value observed.
    fn compare_and_swap(
        &self, key: String, current: Option<Vec<u8>>, new: Vec<u8>, ttl: Option<Duration>,
    ) -> FutureResult<Result<(), Option<Vec<u8>>>>;
}

/// Proxy for a Key-Value bucket.
pub type BucketProxy = omnia_core::Proxy<dyn Bucket>;

/// CAS (Compare-And-Swap) operation handle.
///
/// The snapshot a `wasi:keyvalue/atomics` swap compares against.
#[derive(Clone, Debug)]
pub struct Cas {
    pub(crate) bucket: Arc<dyn Bucket>,
    pub(crate) key: String,
    pub(crate) current: Option<Vec<u8>>,
}
