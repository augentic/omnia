//! Errors

use std::error::Error as StdError;
use std::iter;

use http::StatusCode;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Result type used across the crate.
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Domain level error type returned by the adapter.
///
/// A failed capability call is a [`ServerError`](Self::ServerError) whose
/// code names the class of failure:
///
/// - `not-found`: the binding, container, collection, or required setting
///   does not exist.
/// - `denied`: the provider refused the call.
/// - `invalid`: the input was rejected, such as a zero TTL, or stored bytes
///   failed to decode.
/// - `unsupported`: the request is valid, but this provider cannot serve it.
/// - `other`: any other failure.
///
/// A call the upstream did not answer, or one that exceeded its time bound,
/// is a [`BadGateway`](Self::BadGateway) with the code `unavailable`.
#[derive(Error, Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Error {
    /// Request payload is invalid or missing required fields.
    #[error("code: {code}, description: {description}")]
    BadRequest {
        /// The error code.
        code: String,
        /// The error description.
        description: String,
    },

    /// Resource or data not found.
    #[error("code: {code}, description: {description}")]
    NotFound {
        /// The error code.
        code: String,
        /// The error description.
        description: String,
    },

    /// A non recoverable internal error occurred.
    #[error("code: {code}, description: {description}")]
    ServerError {
        /// The error code.
        code: String,
        /// The error description.
        description: String,
    },

    /// An upstream dependency failed while fulfilling the request.
    #[error("code: {code}, description: {description}")]
    BadGateway {
        /// The error code.
        code: String,
        /// The error description.
        description: String,
    },
}

impl Error {
    /// Create a [`BadRequest`](Self::BadRequest) error carrying `code`.
    ///
    /// The code is the `error` discriminant a client reads from the wire
    /// body, so a specification's own reason code (`"segment_unknown"`) goes
    /// here; [`bad_request!`](crate::bad_request) is the same error under the
    /// default code `bad_request`.
    pub fn bad_request(code: impl Into<String>, description: impl Into<String>) -> Self {
        Self::BadRequest {
            code: code.into(),
            description: description.into(),
        }
    }

    /// Create a [`NotFound`](Self::NotFound) error carrying `code`.
    ///
    /// [`not_found!`](crate::not_found) is the same error under the default
    /// code `not_found`.
    pub fn not_found(code: impl Into<String>, description: impl Into<String>) -> Self {
        Self::NotFound {
            code: code.into(),
            description: description.into(),
        }
    }

    /// Create a [`ServerError`](Self::ServerError) error carrying `code`.
    ///
    /// [`server_error!`](crate::server_error) is the same error under the
    /// default code `server_error`.
    pub fn server_error(code: impl Into<String>, description: impl Into<String>) -> Self {
        Self::ServerError {
            code: code.into(),
            description: description.into(),
        }
    }

    /// Create a [`BadGateway`](Self::BadGateway) error carrying `code`.
    ///
    /// [`bad_gateway!`](crate::bad_gateway) is the same error under the
    /// default code `bad_gateway`.
    pub fn bad_gateway(code: impl Into<String>, description: impl Into<String>) -> Self {
        Self::BadGateway {
            code: code.into(),
            description: description.into(),
        }
    }

    /// Returns the HTTP status code associated with the variant.
    #[must_use]
    pub const fn status(&self) -> StatusCode {
        match self {
            Self::BadRequest { .. } => StatusCode::BAD_REQUEST,
            Self::NotFound { .. } => StatusCode::NOT_FOUND,
            Self::ServerError { .. } => StatusCode::INTERNAL_SERVER_ERROR,
            Self::BadGateway { .. } => StatusCode::BAD_GATEWAY,
        }
    }

    /// Returns the process exit code associated with the variant.
    #[must_use]
    pub const fn exit_code(&self) -> u8 {
        match self {
            Self::BadRequest { .. } => 1,
            Self::NotFound { .. } => 2,
            Self::ServerError { .. } => 3,
            Self::BadGateway { .. } => 4,
        }
    }

    /// Returns the error code for the variant.
    #[must_use]
    pub const fn code(&self) -> &str {
        match self {
            Self::BadRequest { code, .. }
            | Self::NotFound { code, .. }
            | Self::ServerError { code, .. }
            | Self::BadGateway { code, .. } => code.as_str(),
        }
    }

    /// Returns the error description.
    #[must_use]
    pub const fn description(&self) -> &str {
        match self {
            Self::BadRequest { description, .. }
            | Self::NotFound { description, .. }
            | Self::ServerError { description, .. }
            | Self::BadGateway { description, .. } => description.as_str(),
        }
    }

    // The first domain error in the source chain keeps its variant and code,
    // and anything else is a `server_error`. The description is the whole
    // chain, where an `Error` reads as its own description.
    pub(crate) fn from_chain(error: &(dyn StdError + 'static)) -> Self {
        let links = || iter::successors(Some(error), |&link| link.source());
        let description = links()
            .map(|link| {
                link.downcast_ref::<Self>()
                    .map_or_else(|| link.to_string(), |error| error.description().to_owned())
            })
            .collect::<Vec<_>>()
            .join(": ");
        match links().find_map(Self::from_domain) {
            Some(error) => error.with_description(description),
            None => Self::server_error("server_error", description),
        }
    }

    // The `Error` a domain error maps to: this type, or a `vcs`, `plugins`,
    // or `model` error through its own conversion.
    fn from_domain(error: &(dyn StdError + 'static)) -> Option<Self> {
        if let Some(error) = error.downcast_ref::<Self>() {
            return Some(error.clone());
        }
        if let Some(error) = error.downcast_ref::<crate::vcs::Error>() {
            return Some(error.clone().into());
        }
        if let Some(error) = error.downcast_ref::<crate::plugins::Error>() {
            return Some(error.clone().into());
        }
        error.downcast_ref::<crate::model::Error>().map(|error| error.clone().into())
    }

    fn with_description(self, description: String) -> Self {
        match self {
            Self::BadRequest { code, .. } => Self::BadRequest { code, description },
            Self::NotFound { code, .. } => Self::NotFound { code, description },
            Self::ServerError { code, .. } => Self::ServerError { code, description },
            Self::BadGateway { code, .. } => Self::BadGateway { code, description },
        }
    }
}

impl From<anyhow::Error> for Error {
    fn from(err: anyhow::Error) -> Self {
        Self::from_chain(err.as_ref())
    }
}

impl From<serde_json::Error> for Error {
    fn from(err: serde_json::Error) -> Self {
        Self::bad_request("bad_request", err.to_string())
    }
}

/// The transport-neutral wire body of a failed invocation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorBody {
    /// The error code discriminant.
    pub error: String,

    /// The human-readable error description.
    pub message: String,
}

impl From<&Error> for ErrorBody {
    fn from(error: &Error) -> Self {
        Self {
            error: error.code().to_owned(),
            message: error.description().to_owned(),
        }
    }
}

/// Create a `BadRequest` error under the default code `bad_request`.
///
/// Takes a format string with arguments, or one expression whose `Display`
/// form is the description. [`Error::bad_request`] names the code instead.
#[macro_export]
macro_rules! bad_request {
    // A lone format string is also an expr; match the literal first so
    // implicit `{name}` captures reach `format!` instead of `.to_string()`.
    ($fmt:literal $($arg:tt)*) => {
        $crate::Error::bad_request("bad_request", format!($fmt $($arg)*))
    };
    ($err:expr $(,)?) => {
        $crate::Error::bad_request("bad_request", $err.to_string())
    };
}

/// Create a `NotFound` error under the default code `not_found`.
///
/// Takes a format string with arguments, or one expression whose `Display`
/// form is the description. [`Error::not_found`] names the code instead.
#[macro_export]
macro_rules! not_found {
    ($fmt:literal $($arg:tt)*) => {
        $crate::Error::not_found("not_found", format!($fmt $($arg)*))
    };
    ($err:expr $(,)?) => {
        $crate::Error::not_found("not_found", $err.to_string())
    };
}

/// Create a `ServerError` error under the default code `server_error`.
///
/// Takes a format string with arguments, or one expression whose `Display`
/// form is the description. [`Error::server_error`] names the code instead.
#[macro_export]
macro_rules! server_error {
    ($fmt:literal $($arg:tt)*) => {
        $crate::Error::server_error("server_error", format!($fmt $($arg)*))
    };
    ($err:expr $(,)?) => {
        $crate::Error::server_error("server_error", $err.to_string())
    };
}

/// Create a `BadGateway` error under the default code `bad_gateway`.
///
/// Takes a format string with arguments, or one expression whose `Display`
/// form is the description. [`Error::bad_gateway`] names the code instead.
#[macro_export]
macro_rules! bad_gateway {
    ($fmt:literal $($arg:tt)*) => {
        $crate::Error::bad_gateway("bad_gateway", format!($fmt $($arg)*))
    };
    ($err:expr $(,)?) => {
        $crate::Error::bad_gateway("bad_gateway", $err.to_string())
    };
}
