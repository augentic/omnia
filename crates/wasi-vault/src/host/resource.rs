use std::fmt::Debug;

pub use omnia_core::FutureResult;

/// Providers implement the [`Locker`] trait to allow the host to
/// interact with different backend lockers (stores).
pub trait Locker: Debug + Send + Sync + 'static {
    /// Get the value associated with the key.
    fn get(&self, secret_id: String) -> FutureResult<Option<Vec<u8>>>;

    /// Set the value associated with the key.
    fn set(&self, secret_id: String, value: Vec<u8>) -> FutureResult<()>;

    /// Delete the value associated with the key.
    fn delete(&self, secret_id: String) -> FutureResult<()>;

    /// Check if the entry exists.
    fn exists(&self, secret_id: String) -> FutureResult<bool>;

    /// List all secret IDs in the locker.
    fn list_ids(&self) -> FutureResult<Vec<String>>;
}

/// Represents a locker resource in the WASI Vault.
pub type LockerProxy = omnia_core::Proxy<dyn Locker>;
