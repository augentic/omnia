//! The error, result, and listing page a capability call returns.
//!
//! A failed call is an [`Error`] whose [`ErrorKind`] tells the caller what
//! went wrong. Absence is not a failure: a missing key is `Ok(None)`. A
//! listing answers one [`Page`] at a time.

use std::fmt;

/// The result of a capability call.
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// One page of a listing, with the cursor that resumes it.
///
/// Pass [`next`](Self::next) back as the listing's `after` to read the
/// following page. A page without one is the last. A cursor is opaque: it
/// comes from a page and goes back to the listing that made it.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Page<T> {
    /// The items on this page.
    pub items: Vec<T>,

    /// The cursor that resumes the listing, or `None` on the last page.
    pub next: Option<String>,
}

impl<T> Page<T> {
    /// A page holding `items` that resumes at `next`.
    #[must_use]
    pub const fn new(items: Vec<T>, next: Option<String>) -> Self {
        Self { items, next }
    }
}

/// A failed capability call.
///
/// A caller matches on [`kind`](Self::kind). The message is for people, and
/// its text is not part of the contract.
#[derive(Debug, thiserror::Error)]
#[error("{kind}: {message}")]
pub struct Error {
    kind: ErrorKind,
    message: String,
    #[source]
    source: Option<Box<dyn std::error::Error + Send + Sync>>,
}

impl Error {
    /// Create an error of `kind` described by `message`.
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            source: None,
        }
    }

    /// Attach the error that caused this one.
    #[must_use]
    pub fn with_source(
        mut self, source: impl Into<Box<dyn std::error::Error + Send + Sync>>,
    ) -> Self {
        self.source = Some(source.into());
        self
    }

    /// The kind of failure.
    #[must_use]
    pub const fn kind(&self) -> ErrorKind {
        self.kind
    }
}

/// What kind of failure a capability call met.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ErrorKind {
    /// The binding, container, collection, or required setting does not exist.
    NotFound,

    /// The provider refused the call.
    Denied,

    /// The input was rejected, such as a zero TTL, or stored bytes failed to decode.
    Invalid,

    /// The request is valid, but this provider cannot serve it.
    Unsupported,

    /// The upstream did not answer, or the call exceeded its time bound.
    Unavailable,

    /// Any other failure.
    Other,
}

impl fmt::Display for ErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NotFound => "not found",
            Self::Denied => "denied",
            Self::Invalid => "invalid",
            Self::Unsupported => "unsupported",
            Self::Unavailable => "unavailable",
            Self::Other => "other",
        })
    }
}
