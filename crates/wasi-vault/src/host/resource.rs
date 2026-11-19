use std::fmt::Debug;

pub use omnia_core::FutureResult;

/// Providers implement the [`Locker`] trait to allow the host to
/// interact with different backend lockers (stores).
///
/// A backend stores every portable name and value, and fails one it cannot
/// store with [`Error::Invalid`](crate::Error::Invalid):
///
/// - a portable secret name is 1 to 63 characters long and holds only
///   lowercase ASCII letters, digits, and `-`;
/// - a portable value is not empty and at most 16 `KiB`.
///
/// A backend picks the kind of a failure by returning an
/// [`Error`](crate::Error), and any other error reaches the guest as
/// [`Error::Other`](crate::Error::Other).
pub trait Locker: Debug + Send + Sync + 'static {
    /// The value of the secret `name`, or `None` when it is absent.
    fn get(&self, name: String) -> FutureResult<Option<Vec<u8>>>;

    /// Store `value` as the secret `name`, replacing any value there.
    fn set(&self, name: String, value: Vec<u8>) -> FutureResult<()>;

    /// Delete the secret `name`, succeeding when it is already absent.
    ///
    /// The name may be set again at once.
    fn delete(&self, name: String) -> FutureResult<()>;

    /// Whether the secret `name` exists.
    fn exists(&self, name: String) -> FutureResult<bool>;

    /// The name of every secret in the locker, in any order.
    fn list_ids(&self) -> FutureResult<Vec<String>>;
}

/// The host's side of a guest's `locker` handle.
pub type LockerProxy = omnia_core::Proxy<dyn Locker>;
