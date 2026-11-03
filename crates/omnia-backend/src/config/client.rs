//! Service contract for `wasi:config` backends.

use std::fmt::Debug;

use crate::FutureResult;

/// Service contract for `wasi:config` backends.
pub trait ConfigClient: Debug + Send + Sync + 'static {
    /// Fetch a configuration value by `key`; `Ok(None)` when absent.
    fn get(&self, key: String) -> FutureResult<Option<String>>;

    /// Fetch all configuration key-value pairs.
    fn get_all(&self) -> FutureResult<Vec<(String, String)>>;
}
