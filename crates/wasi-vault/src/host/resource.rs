use std::fmt::Debug;
use std::ops::Deref;
use std::sync::Arc;

pub use yetti::FutureResult;

/// Providers implement the [`Locker`] trait to allow the host to
/// interact with different backend lockers (stores).
pub trait Locker: Debug + Send + Sync + 'static {
    /// The name of the locker.
    fn identifier(&self) -> String;

    /// Get the value associated with the key.
    fn get(&self, secret_id: String) -> FutureResult<Option<Vec<u8>>>;

    /// Set the value associated with the key.
    fn set(&self, secret_id: String, value: Vec<u8>) -> FutureResult<()>;

    /// Delete the value associated with the key.
    fn delete(&self, secret_id: String) -> FutureResult<()>;

    /// Check if the entry exists.
    fn exists(&self, secret_id: String) -> FutureResult<bool>;

    /// List all keys in the bucket.
    fn list_ids(&self) -> FutureResult<Vec<String>>;
}

/// Represents a locker resource in the WASI Vault.
#[derive(Debug, Clone)]
pub struct LockerProxy(pub Arc<dyn Locker>);

impl Deref for LockerProxy {
    type Target = Arc<dyn Locker>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
