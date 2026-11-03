//! Service contract for `omnia:vault` backends.

use std::fmt::Debug;
use std::sync::Arc;

use crate::FutureResult;

/// Service contract for `omnia:vault` backends.
pub trait VaultClient: Debug + Send + Sync + 'static {
    /// Open a locker by identifier.
    fn open_locker(&self, identifier: String) -> FutureResult<Arc<dyn Locker>>;
}

/// Stateful handle to a secret locker.
pub trait Locker: Debug + Send + Sync + 'static {
    /// Get the secret value associated with `secret_id`.
    fn get(&self, secret_id: String) -> FutureResult<Option<Vec<u8>>>;

    /// Store `value` under `secret_id`.
    fn set(&self, secret_id: String, value: Vec<u8>) -> FutureResult<()>;

    /// Delete `secret_id`.
    fn delete(&self, secret_id: String) -> FutureResult<()>;

    /// Check whether `secret_id` exists.
    fn exists(&self, secret_id: String) -> FutureResult<bool>;

    /// List all secret IDs in the locker.
    fn list_ids(&self) -> FutureResult<Vec<String>>;
}
