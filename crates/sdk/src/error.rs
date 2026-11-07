//! Errors

use http::StatusCode;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Result type used across the crate.
pub type Result<T> = std::result::Result<T, Error>;

/// Domain level error type returned by the adapter.
#[derive(Error, Debug, Clone, Serialize, Deserialize)]
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
    pub fn code(&self) -> String {
        match self {
            Self::BadRequest { code, .. }
            | Self::NotFound { code, .. }
            | Self::ServerError { code, .. }
            | Self::BadGateway { code, .. } => code.clone(),
        }
    }

    /// Returns the error description.
    #[must_use]
    pub fn description(&self) -> String {
        match self {
            Self::BadRequest { description, .. }
            | Self::NotFound { description, .. }
            | Self::ServerError { description, .. }
            | Self::BadGateway { description, .. } => description.clone(),
        }
    }
}

// An `Error` inside the chain keeps its variant and code and gains the
// context wrapped around it; anything else is a `server_error`.
impl From<anyhow::Error> for Error {
    fn from(err: anyhow::Error) -> Self {
        let chain = err.chain().map(ToString::to_string).collect::<Vec<_>>().join(": ");

        if let Some(inner) = err.downcast_ref::<Self>() {
            tracing::debug!("Error: {err}, caused by: {inner}");

            return match inner {
                Self::BadRequest { code, .. } => Self::bad_request(code, chain),
                Self::NotFound { code, .. } => Self::not_found(code, chain),
                Self::ServerError { code, .. } => Self::server_error(code, chain),
                Self::BadGateway { code, .. } => Self::bad_gateway(code, chain),
            };
        }

        Self::server_error("server_error", chain)
    }
}

impl From<serde_json::Error> for Error {
    fn from(err: serde_json::Error) -> Self {
        Self::bad_request("bad_request", err.to_string())
    }
}

#[cfg(feature = "sql")]
impl From<omnia_backend::sql::ColumnError> for Error {
    fn from(err: omnia_backend::sql::ColumnError) -> Self {
        Self::server_error("server_error", err.to_string())
    }
}

impl From<crate::model::Error> for Error {
    fn from(err: crate::model::Error) -> Self {
        use crate::model::Error as Model;
        match err {
            // the guest's own request, or findings the model could not resolve
            Model::InvalidRequest(_) | Model::BudgetExhausted(_) => crate::bad_request!(err),
            // a tool, or the transport
            Model::ToolFailed(_) | Model::Backend(_) => crate::bad_gateway!(err),
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
