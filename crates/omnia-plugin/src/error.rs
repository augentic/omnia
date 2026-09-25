//! The loader's refusal vocabulary, owned here so every module below the WIT
//! adapter depends downward on plain Rust rather than upward on bindgen
//! output.

use std::fmt;

/// Why a load was refused — the host mirror of the `omnia:plugins/loader`
/// `error` variant.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoadError {
    /// The request or deployment is wrong and a retry cannot succeed.
    Refused(String),
    /// The guest's source could not produce its bytes; a retry may succeed.
    Unavailable(String),
    /// Loader misconfiguration or an internal registration failure.
    Internal(String),
}

impl LoadError {
    // The refusal for a runtime that linked the loader host but installed no
    // `Plugins` extension — a runtime assembled by hand from parts, since
    // `Deployment::assemble` always installs one.
    pub(crate) fn no_plugins(from: impl fmt::Display) -> Self {
        Self::Internal(format!(
            "this runtime has no guest loader installed; loading `{from}` needs \
             `Plugins::install` on the assembled runtime"
        ))
    }
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Refused(detail) => write!(f, "refused: {detail}"),
            Self::Unavailable(detail) => write!(f, "unavailable: {detail}"),
            Self::Internal(detail) => write!(f, "internal: {detail}"),
        }
    }
}

impl std::error::Error for LoadError {}
