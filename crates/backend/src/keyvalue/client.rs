//! Service contract for `wasi:keyvalue` backends.

use std::fmt::Debug;
use std::sync::Arc;
use std::time::Duration;

use crate::FutureResult;

/// Service contract for `wasi:keyvalue` backends.
pub trait KeyValueClient: Debug + Send + Sync + 'static {
    /// Open a bucket.
    fn open_bucket(&self, identifier: String) -> FutureResult<Arc<dyn Bucket>>;
}

/// Stateful handle to a key-value bucket.
pub trait Bucket: Debug + Send + Sync + 'static {
    /// Get the value associated with `key`.
    fn get(&self, key: String) -> FutureResult<Option<Vec<u8>>>;

    /// Set the value associated with `key`, optionally expiring after `ttl`.
    fn set(&self, key: String, value: Vec<u8>, ttl: Option<Duration>) -> FutureResult<()>;

    /// Delete the value associated with `key`.
    fn delete(&self, key: String) -> FutureResult<()>;

    /// Check whether `key` exists.
    fn exists(&self, key: String) -> FutureResult<bool>;

    /// List all keys in the bucket.
    fn keys(&self) -> FutureResult<Vec<String>>;

    /// Atomically increment the 8-byte big-endian signed integer at `key`
    /// by `delta` (starting at zero when absent).
    fn increment(&self, key: String, delta: i64) -> FutureResult<i64>;

    /// Single-step atomic compare-and-swap: replace the value at `key` with
    /// `value` only when the stored value equals `expected` (`None` means
    /// absent), preserving any remaining TTL. On mismatch returns
    /// `Err(observed)` carrying the value currently stored.
    fn cas(
        &self, key: String, expected: Option<Vec<u8>>, value: Vec<u8>,
    ) -> FutureResult<Result<(), Option<Vec<u8>>>>;
}
