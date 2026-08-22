use std::fmt::Debug;
use std::sync::Arc;

pub use omnia::FutureResult;

/// Providers implement the [`Bucket`] trait to allow the host to
/// interact with different backend buckets (stores).
pub trait Bucket: Debug + Send + Sync + 'static {
    /// Get the value associated with the key.
    fn get(&self, key: String) -> FutureResult<Option<Vec<u8>>>;

    /// Set the value associated with the key.
    fn set(&self, key: String, value: Vec<u8>) -> FutureResult<()>;

    /// Delete the value associated with the key.
    fn delete(&self, key: String) -> FutureResult<()>;

    /// Check if the entry exists.
    fn exists(&self, key: String) -> FutureResult<bool>;

    /// List all keys in the bucket.
    fn keys(&self) -> FutureResult<Vec<String>>;

    /// Native atomic increment, if the backend has one (e.g. Redis `INCRBY`).
    fn increment(&self, key: String, delta: i64) -> FutureResult<i64>;

    /// Atomic swap while the value still matches the handle's snapshot; a
    /// stale handle returns refreshed at the observed value.
    fn swap(&self, cas: Cas, value: Vec<u8>) -> FutureResult<Result<(), Cas>>;
}

/// Proxy for a Key-Value bucket.
pub type BucketProxy = omnia::Proxy<dyn Bucket>;

/// CAS (Compare-And-Swap) operation handle.
#[derive(Clone, Debug)]
pub struct Cas {
    /// The bucket the operation reads from and swaps into.
    pub bucket: Arc<dyn Bucket>,

    /// The key associated with the CAS operation.
    pub key: String,

    /// The current value associated with the key.
    pub current: Option<Vec<u8>>,
}
