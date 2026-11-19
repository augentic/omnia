use std::fmt::Debug;
use std::time::Duration;

use omnia_core::Digest;
pub use omnia_core::FutureResult;

/// Providers implement the [`Bucket`] trait to allow the host to
/// interact with different backend buckets (stores).
///
/// Every write is atomic, and a precondition is checked in the same step as
/// the write it guards. A [`Revision`] changes whenever an entry's bytes
/// change, and a backend that keeps none of its own reports
/// [`Revision::of`] the stored bytes.
///
/// A backend stores every portable key, and fails a key it cannot store with
/// [`Error::Invalid`](crate::Error::Invalid). A portable key:
///
/// - is one or more segments joined by `.`;
/// - has no empty segment, and holds only ASCII letters, digits, `_`, and
///   `-` in each;
/// - is at most 255 bytes long.
///
/// The host rejects a zero TTL before a backend sees it. A backend picks the
/// kind of a failure by returning an [`Error`](crate::Error), and any other
/// error reaches the guest as [`Error::Other`](crate::Error::Other).
pub trait Bucket: Debug + Send + Sync + 'static {
    /// The entry at `key`, or `None` when it is absent.
    fn get(&self, key: String) -> FutureResult<Option<Entry>>;

    /// Store `value` at `key` with `lifetime`, and with `when` only while it
    /// holds, returning the revision written.
    ///
    /// A precondition that does not hold changes nothing and returns the
    /// entry found, or `None` when the key is absent.
    fn set(
        &self, key: String, value: Vec<u8>, lifetime: Lifetime, when: Option<Precondition>,
    ) -> FutureResult<Result<Revision, Option<Entry>>>;

    /// Delete the entry at `key`, and with `when` only while it has that
    /// revision.
    ///
    /// Without `when`, deleting an absent key succeeds. A revision that does
    /// not match changes nothing and returns the entry found, or `None` when
    /// the key is absent.
    fn delete(
        &self, key: String, when: Option<Revision>,
    ) -> FutureResult<Result<(), Option<Entry>>>;

    /// Check if the entry exists.
    fn exists(&self, key: String) -> FutureResult<bool>;

    /// List all keys in the bucket.
    fn keys(&self) -> FutureResult<Vec<String>>;

    /// Atomically add `delta` to the integer at `key`, starting from zero,
    /// and return the new value.
    ///
    /// `ttl` applies only when this creates the counter. An existing counter
    /// keeps the time it has left.
    fn increment(&self, key: String, delta: i64, ttl: Option<Duration>) -> FutureResult<i64>;
}

/// A stored value and the revision of the state that holds it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// The stored bytes.
    pub value: Vec<u8>,
    /// The revision of the stored state.
    pub revision: Revision,
}

/// A token for one stored state of an entry, compared only for equality.
///
/// It changes whenever the entry's bytes change. Two states that hold the
/// same bytes may share one.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Revision(String);

impl Revision {
    /// The revision of a state that holds `value`, for a backend that keeps
    /// no revision of its own.
    ///
    /// Two states that hold the same bytes share it.
    #[must_use]
    pub fn of(value: &[u8]) -> Self {
        Self(Digest::of(value).to_string())
    }

    /// The token as the guest sees it.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<String> for Revision {
    fn from(token: String) -> Self {
        Self(token)
    }
}

impl From<Revision> for String {
    fn from(revision: Revision) -> Self {
        revision.0
    }
}

/// What a write does to an entry's lifetime.
///
/// A TTL counts from the write and rounds up to the store's resolution, so
/// an entry never expires early. With no TTL, an entry never expires.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lifetime {
    /// The entry expires after the TTL, whatever time it had left.
    Replace(Option<Duration>),
    /// An existing entry keeps the time it has left, and a new one expires
    /// after the TTL.
    Keep(Option<Duration>),
}

/// What must hold for a write to take effect.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Precondition {
    /// No entry is stored at the key.
    Absent,
    /// The entry stored at the key has this revision.
    Matches(Revision),
}

impl Precondition {
    /// Whether the precondition holds over `current`, the revision stored at
    /// the key, or `None` when the key is absent.
    #[must_use]
    pub fn holds(&self, current: Option<&Revision>) -> bool {
        match self {
            Self::Absent => current.is_none(),
            Self::Matches(revision) => current == Some(revision),
        }
    }
}
