//! Errors

use std::error::Error as StdError;
use std::iter;

use http::StatusCode;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::bindings::imports::omnia::types::types;
use crate::{model, plugins, vcs};

/// Result type used across the crate.
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Domain level error type returned by the adapter.
///
/// An error is a [`kind`](Self::kind), a [`code`](Self::code) a client
/// matches on, and a [`description`](Self::description). The kind fixes the
/// HTTP [`status`](Self::status) and the process
/// [`exit_code`](Self::exit_code).
///
/// A failed capability call is a [`ServerError`](ErrorKind::ServerError)
/// whose code names the class of failure:
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
/// is a [`BadGateway`](ErrorKind::BadGateway) with the code `unavailable`.
///
/// A typed outcome a call answers with instead of its result is an error
/// under the outcome's name. A completion that spent a budget,
/// [`model::Exhausted`], is a [`BadRequest`](ErrorKind::BadRequest) with the
/// code `exhausted`. The others are a [`Conflict`](ErrorKind::Conflict):
///
/// - `conflict`: a load's precondition did not hold, [`plugins::Conflict`].
/// - `occupied`: a place already holds something, [`vcs::Occupied`].
/// - `diverged`: a remote moved past a push, [`vcs::Diverged`].
/// - `conflicts` or `pending`: a merge sealed nothing, [`vcs::Unmerged`].
#[derive(Error, Debug, Clone, PartialEq, Eq)]
#[error("code: {code}, description: {description}")]
pub struct Error {
    kind: ErrorKind,
    code: String,
    description: String,
}

impl Error {
    /// Create an error of `kind` carrying `code`.
    ///
    /// The code is the `error` discriminant a client reads from the wire
    /// body. The constructor named for each kind is the everyday form.
    pub fn new(kind: ErrorKind, code: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            kind,
            code: code.into(),
            description: description.into(),
        }
    }

    /// Create a [`BadRequest`](ErrorKind::BadRequest) error carrying `code`.
    ///
    /// The code is the `error` discriminant a client reads from the wire
    /// body, so a specification's own reason code (`"segment_unknown"`) goes
    /// here; [`bad_request!`](crate::bad_request) is the same error under the
    /// default code `bad_request`.
    pub fn bad_request(code: impl Into<String>, description: impl Into<String>) -> Self {
        Self::new(ErrorKind::BadRequest, code, description)
    }

    /// Create a [`NotFound`](ErrorKind::NotFound) error carrying `code`.
    ///
    /// [`not_found!`](crate::not_found) is the same error under the default
    /// code `not_found`.
    pub fn not_found(code: impl Into<String>, description: impl Into<String>) -> Self {
        Self::new(ErrorKind::NotFound, code, description)
    }

    /// Create a [`Conflict`](ErrorKind::Conflict) error carrying `code`.
    ///
    /// [`conflict!`](crate::conflict) is the same error under the default
    /// code `conflict`.
    pub fn conflict(code: impl Into<String>, description: impl Into<String>) -> Self {
        Self::new(ErrorKind::Conflict, code, description)
    }

    /// Create a [`ServerError`](ErrorKind::ServerError) error carrying `code`.
    ///
    /// [`server_error!`](crate::server_error) is the same error under the
    /// default code `server_error`.
    pub fn server_error(code: impl Into<String>, description: impl Into<String>) -> Self {
        Self::new(ErrorKind::ServerError, code, description)
    }

    /// Create a [`BadGateway`](ErrorKind::BadGateway) error carrying `code`.
    ///
    /// [`bad_gateway!`](crate::bad_gateway) is the same error under the
    /// default code `bad_gateway`.
    pub fn bad_gateway(code: impl Into<String>, description: impl Into<String>) -> Self {
        Self::new(ErrorKind::BadGateway, code, description)
    }

    /// Returns the kind of the error.
    #[must_use]
    pub const fn kind(&self) -> ErrorKind {
        self.kind
    }

    /// Returns the HTTP status code associated with the kind.
    #[must_use]
    pub const fn status(&self) -> StatusCode {
        self.kind.status()
    }

    /// Returns the process exit code associated with the kind.
    #[must_use]
    pub const fn exit_code(&self) -> u8 {
        self.kind.exit_code()
    }

    /// Returns the error code.
    #[must_use]
    pub const fn code(&self) -> &str {
        self.code.as_str()
    }

    /// Returns the error description.
    #[must_use]
    pub const fn description(&self) -> &str {
        self.description.as_str()
    }

    // The first domain error in the source chain keeps its kind and code, and
    // anything else is a `server_error`. The description is the whole
    // chain.
    pub(crate) fn from_chain(error: &(dyn StdError + 'static)) -> Self {
        let links = || iter::successors(Some(error), |&link| link.source());
        let description = links().map(Self::describe).collect::<Vec<_>>().join(": ");
        match links().find_map(Self::from_domain) {
            Some(error) => error.with_description(description),
            None => Self::server_error("server_error", description),
        }
    }

    // An `Error` reads as its own description and a wire error as its
    // message, since the generated `Display` is the `Debug` form.
    fn describe(link: &(dyn StdError + 'static)) -> String {
        if let Some(error) = link.downcast_ref::<Self>() {
            return error.description().to_owned();
        }
        if let Some(error) = link.downcast_ref::<types::Error>() {
            return error.message().to_owned();
        }
        link.to_string()
    }

    // The `Error` a domain error maps to: this type, or a wire error or a
    // typed outcome through its own conversion.
    fn from_domain(error: &(dyn StdError + 'static)) -> Option<Self> {
        if let Some(error) = error.downcast_ref::<Self>() {
            return Some(error.clone());
        }
        if let Some(error) = error.downcast_ref::<types::Error>() {
            return Some(error.clone().into());
        }
        if let Some(outcome) = error.downcast_ref::<model::Exhausted>() {
            return Some(outcome.clone().into());
        }
        if let Some(outcome) = error.downcast_ref::<plugins::Conflict>() {
            return Some(outcome.clone().into());
        }
        if let Some(outcome) = error.downcast_ref::<vcs::Occupied>() {
            return Some(outcome.clone().into());
        }
        if let Some(outcome) = error.downcast_ref::<vcs::Diverged>() {
            return Some(outcome.clone().into());
        }
        error.downcast_ref::<vcs::Unmerged>().map(|outcome| outcome.clone().into())
    }

    fn with_description(self, description: String) -> Self {
        Self { description, ..self }
    }
}

/// The class of an [`Error`](struct@Error), which fixes its HTTP status and exit code.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ErrorKind {
    /// Request payload is invalid or missing required fields.
    BadRequest,

    /// Resource or data not found.
    NotFound,

    /// The request conflicts with the current state of its target.
    Conflict,

    /// A non recoverable internal error occurred.
    ServerError,

    /// An upstream dependency failed while fulfilling the request.
    BadGateway,
}

impl ErrorKind {
    /// Returns the HTTP status code of an error of this kind.
    #[must_use]
    pub const fn status(self) -> StatusCode {
        match self {
            Self::BadRequest => StatusCode::BAD_REQUEST,
            Self::NotFound => StatusCode::NOT_FOUND,
            Self::Conflict => StatusCode::CONFLICT,
            Self::ServerError => StatusCode::INTERNAL_SERVER_ERROR,
            Self::BadGateway => StatusCode::BAD_GATEWAY,
        }
    }

    /// Returns the process exit code of an error of this kind.
    #[must_use]
    pub const fn exit_code(self) -> u8 {
        match self {
            Self::BadRequest => 1,
            Self::NotFound => 2,
            Self::ServerError => 3,
            Self::BadGateway => 4,
            Self::Conflict => 5,
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

impl From<http::Error> for Error {
    fn from(err: http::Error) -> Self {
        Self::server_error("invalid", err.to_string())
    }
}

impl From<types::Error> for Error {
    fn from(err: types::Error) -> Self {
        match err {
            types::Error::NotFound(message) => Self::server_error("not-found", message),
            types::Error::Denied(message) => Self::server_error("denied", message),
            types::Error::Invalid(message) => Self::server_error("invalid", message),
            types::Error::Unsupported(message) => Self::server_error("unsupported", message),
            types::Error::Unavailable(message) => Self::bad_gateway("unavailable", message),
            types::Error::Other(message) => Self::server_error("other", message),
        }
    }
}

impl From<model::Exhausted> for Error {
    fn from(exhausted: model::Exhausted) -> Self {
        Self::bad_request("exhausted", exhausted.to_string())
    }
}

impl From<plugins::Conflict> for Error {
    fn from(conflict: plugins::Conflict) -> Self {
        Self::conflict("conflict", conflict.to_string())
    }
}

impl From<vcs::Occupied> for Error {
    fn from(occupied: vcs::Occupied) -> Self {
        Self::conflict("occupied", occupied.to_string())
    }
}

impl From<vcs::Diverged> for Error {
    fn from(diverged: vcs::Diverged) -> Self {
        Self::conflict("diverged", diverged.to_string())
    }
}

impl From<vcs::Unmerged> for Error {
    fn from(unmerged: vcs::Unmerged) -> Self {
        let code = match &unmerged {
            vcs::Unmerged::Conflicts(_) => "conflicts",
            vcs::Unmerged::Pending(_) => "pending",
        };
        Self::conflict(code, unmerged.to_string())
    }
}

// a call with no precondition has none to fail, so a conflict is the host's
pub fn unconditional<T, C>(outcome: Result<T, C>) -> Result<T> {
    outcome.map_err(|_conflict| {
        Error::server_error("other", "the host reported a conflict on an unconditional call")
    })
}

impl types::Error {
    fn message(&self) -> &str {
        match self {
            Self::NotFound(message)
            | Self::Denied(message)
            | Self::Invalid(message)
            | Self::Unsupported(message)
            | Self::Unavailable(message)
            | Self::Other(message) => message,
        }
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

/// Create a `Conflict` error under the default code `conflict`.
///
/// Takes a format string with arguments, or one expression whose `Display`
/// form is the description. [`Error::conflict`] names the code instead.
#[macro_export]
macro_rules! conflict {
    ($fmt:literal $($arg:tt)*) => {
        $crate::Error::conflict("conflict", format!($fmt $($arg)*))
    };
    ($err:expr $(,)?) => {
        $crate::Error::conflict("conflict", $err.to_string())
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
