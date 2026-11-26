//! The loader's failure vocabulary, owned here so every module below the WIT
//! adapter depends downward on plain Rust rather than upward on bindgen
//! output.

use std::fmt;

use omnia_core::GuestError;

/// Why a load failed: the host form of the `omnia:plugins/types` `error`
/// variant.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum LoadError {
    /// The source names nothing. No guest is declared under the name, no
    /// file is at the path, or nothing routes the package or holds the
    /// release.
    #[error("not-found: {0}")]
    NotFound(String),
    /// The deployment or the loading guest's grants do not allow the load.
    ///
    /// Refused loads include:
    ///
    /// - a path beneath a writable mount, or beneath a mount the loading
    ///   guest does not reach;
    /// - a path or package deriving a name the deployment declares;
    /// - a package whose fetch from the registry the load names leads to a
    ///   host the loading guest does not reach;
    /// - a component importing code from a package its grants do not reach.
    #[error("denied: {0}")]
    Denied(String),
    /// The request was rejected, such as a malformed path or digest, or the
    /// bytes are not a component the deployment can run.
    #[error("invalid: {0}")]
    Invalid(String),
    /// A valid request this runtime cannot serve, such as a load on a
    /// runtime with no loader installed.
    #[error("unsupported: {0}")]
    Unsupported(String),
    /// The mount, the store, or the registry did not produce the bytes. A
    /// retry may succeed.
    #[error("unavailable: {0}")]
    Unavailable(String),
    /// Any other failure, such as a registration that failed.
    #[error("other: {0}")]
    Other(String),
}

impl LoadError {
    // A runtime that linked the loader host but installed no `Plugins`
    // extension, which `Deployment::assemble` always installs.
    pub(crate) fn no_plugins(from: impl fmt::Display) -> Self {
        Self::Unsupported(format!(
            "this runtime has no guest loader installed; loading `{from}` needs a runtime \
             assembled by `Deployment::assemble`"
        ))
    }
}

// The admission lifts a conflict into a `LoadConflict`, so one reaching this
// conversion is a fault.
impl From<GuestError> for LoadError {
    fn from(error: GuestError) -> Self {
        match error {
            GuestError::Unregistered(id) => {
                Self::NotFound(format!("no guest `{id}` is declared by this deployment"))
            }
            GuestError::NotFound(detail) => Self::NotFound(detail),
            GuestError::Denied(detail) => Self::Denied(detail),
            GuestError::Invalid(detail) => Self::Invalid(detail),
            GuestError::Unsupported(detail) => Self::Unsupported(detail),
            GuestError::Unavailable(detail) => Self::Unavailable(detail),
            GuestError::Conflict { detail, .. } | GuestError::Internal(detail) => {
                Self::Other(detail)
            }
        }
    }
}
