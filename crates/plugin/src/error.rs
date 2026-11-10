//! The loader's refusal vocabulary, owned here so every module below the WIT
//! adapter depends downward on plain Rust rather than upward on bindgen
//! output.

use std::fmt;

use omnia_core::GuestError;

/// Why a load was refused — the host mirror of the `omnia:plugins/loader`
/// `error` variant.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum LoadError {
    /// The request or deployment is wrong and a retry cannot succeed.
    #[error("refused: {0}")]
    Refused(String),
    /// The guest's source could not produce its bytes; a retry may succeed.
    #[error("unavailable: {0}")]
    Unavailable(String),
    /// Loader misconfiguration or an internal registration failure.
    #[error("internal: {0}")]
    Internal(String),
}

impl LoadError {
    /// The refusal for a runtime that linked the loader host but installed no
    /// `Plugins` extension: one assembled by hand from parts.
    pub(crate) fn no_plugins(from: impl fmt::Display) -> Self {
        Self::Internal(format!(
            "this runtime has no guest loader installed; loading `{from}` needs \
             `Plugins::install` on the assembled runtime"
        ))
    }
}

impl From<GuestError> for LoadError {
    fn from(error: GuestError) -> Self {
        match error {
            GuestError::Unregistered(id) => {
                Self::Refused(format!("no guest `{id}` is declared by this deployment"))
            }
            GuestError::Refused(detail) => Self::Refused(detail),
            GuestError::Unavailable(detail) => Self::Unavailable(detail),
            GuestError::Internal(detail) => Self::Internal(detail),
        }
    }
}
