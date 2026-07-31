//! Errors

use std::fmt;

use http::StatusCode;
use serde::{Deserialize, Serialize};

/// Result type used across the crate.
pub type Result<T> = std::result::Result<T, Error>;

/// The protocol mapping of an [`Error`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ErrorKind {
    /// Request payload is invalid or missing required fields.
    BadRequest,
    /// Resource or data not found.
    NotFound,
    /// A non recoverable internal error occurred.
    ServerError,
    /// An upstream dependency failed while fulfilling the request.
    BadGateway,
    /// A domain-controlled error carrying a JSON body; the code is the HTTP
    /// status.
    Json,
}

/// Domain level error type returned by the adapter.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(into = "Wire", from = "Wire")]
pub struct Error {
    kind: ErrorKind,
    code: String,
    description: String,
    body: Option<serde_json::Value>,
}

impl Error {
    /// Create an error with an explicit code.
    #[must_use]
    pub fn new(kind: ErrorKind, code: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            kind,
            code: code.into(),
            description: description.into(),
            body: None,
        }
    }

    /// Create a domain-controlled JSON error; `code` is the HTTP status the
    /// response renders with.
    #[must_use]
    pub fn json(code: impl Into<String>, body: serde_json::Value) -> Self {
        let code = code.into();
        Self {
            kind: ErrorKind::Json,
            description: code.clone(),
            code,
            body: Some(body),
        }
    }

    /// Returns the protocol mapping of the error.
    #[must_use]
    pub const fn kind(&self) -> ErrorKind {
        self.kind
    }

    /// Returns the HTTP status code associated with the error.
    #[must_use]
    pub fn status(&self) -> StatusCode {
        match self.kind {
            ErrorKind::BadRequest => StatusCode::BAD_REQUEST,
            ErrorKind::NotFound => StatusCode::NOT_FOUND,
            ErrorKind::ServerError => StatusCode::INTERNAL_SERVER_ERROR,
            ErrorKind::BadGateway => StatusCode::BAD_GATEWAY,
            ErrorKind::Json => self
                .code
                .parse::<u16>()
                .ok()
                .and_then(|n| StatusCode::from_u16(n).ok())
                .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
        }
    }

    /// Returns the error code.
    #[must_use]
    pub fn code(&self) -> String {
        self.code.clone()
    }

    /// Returns the error description.
    #[must_use]
    pub fn description(&self) -> String {
        self.description.clone()
    }

    /// Returns the JSON body of a domain-controlled JSON error.
    #[must_use]
    pub fn json_body(&self) -> Option<serde_json::Value> {
        self.body.clone()
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.kind == ErrorKind::Json {
            write!(f, "code: {}", self.code)
        } else {
            write!(f, "code: {}, description: {}", self.code, self.description)
        }
    }
}

impl std::error::Error for Error {}

impl From<anyhow::Error> for Error {
    fn from(err: anyhow::Error) -> Self {
        let chain = err.chain().map(ToString::to_string).collect::<Vec<_>>().join(": ");

        // A domain error keeps its kind/code (and JSON body) and gains the
        // accumulated context as its description.
        if let Some(inner) = err.downcast_ref::<Self>() {
            tracing::debug!("Error: {err}, caused by: {inner}");
            let mut error = inner.clone();
            if error.kind != ErrorKind::Json {
                error.description = chain;
            }
            return error;
        }

        Self::new(ErrorKind::ServerError, "server_error", chain)
    }
}

impl From<serde_json::Error> for Error {
    fn from(err: serde_json::Error) -> Self {
        Self::new(ErrorKind::BadRequest, "serde_json", err.to_string())
    }
}

/// The serialized shape: an externally tagged enum, kept byte-compatible with
/// stored fixtures and wire payloads produced before [`Error`] became a
/// struct.
#[derive(Clone, Serialize, Deserialize)]
enum Wire {
    BadRequest { code: String, description: String },
    NotFound { code: String, description: String },
    ServerError { code: String, description: String },
    BadGateway { code: String, description: String },
    Json { code: String, body: serde_json::Value },
}

impl From<Error> for Wire {
    fn from(error: Error) -> Self {
        let Error {
            kind,
            code,
            description,
            body,
        } = error;
        match kind {
            ErrorKind::BadRequest => Self::BadRequest { code, description },
            ErrorKind::NotFound => Self::NotFound { code, description },
            ErrorKind::ServerError => Self::ServerError { code, description },
            ErrorKind::BadGateway => Self::BadGateway { code, description },
            ErrorKind::Json => Self::Json {
                code,
                body: body.unwrap_or(serde_json::Value::Null),
            },
        }
    }
}

impl From<Wire> for Error {
    fn from(wire: Wire) -> Self {
        match wire {
            Wire::BadRequest { code, description } => {
                Self::new(ErrorKind::BadRequest, code, description)
            }
            Wire::NotFound { code, description } => {
                Self::new(ErrorKind::NotFound, code, description)
            }
            Wire::ServerError { code, description } => {
                Self::new(ErrorKind::ServerError, code, description)
            }
            Wire::BadGateway { code, description } => {
                Self::new(ErrorKind::BadGateway, code, description)
            }
            Wire::Json { code, body } => Self::json(code, body),
        }
    }
}

/// Create an [`Error`] of the given [`ErrorKind`] with its conventional code.
#[doc(hidden)]
#[macro_export]
macro_rules! __guest_error {
    ($kind:ident, $code:literal, $($arg:tt)+) => {
        $crate::Error::new($crate::ErrorKind::$kind, $code, format!($($arg)+))
    };
}

/// Create a new `BadRequest` error.
#[macro_export]
macro_rules! bad_request {
    ($($arg:tt)+) => { $crate::__guest_error!(BadRequest, "bad_request", $($arg)+) };
}

/// Create a new `NotFound` error.
#[macro_export]
macro_rules! not_found {
    ($($arg:tt)+) => { $crate::__guest_error!(NotFound, "not_found", $($arg)+) };
}

/// Create a new `ServerError` error.
#[macro_export]
macro_rules! server_error {
    ($($arg:tt)+) => { $crate::__guest_error!(ServerError, "server_error", $($arg)+) };
}

/// Create a new `BadGateway` error.
#[macro_export]
macro_rules! bad_gateway {
    ($($arg:tt)+) => { $crate::__guest_error!(BadGateway, "bad_gateway", $($arg)+) };
}

#[cfg(test)]
mod tests {
    use anyhow::{Context, Result, anyhow};
    use http::StatusCode;
    use serde_json::Value;
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;
    use tracing_subscriber::{EnvFilter, Registry, fmt};

    use super::Error;

    #[test]
    fn with_context() {
        Registry::default().with(EnvFilter::new("debug")).with(fmt::layer()).init();

        let context_error = || -> Result<(), Error> {
            Err(bad_request!("invalid input"))
                .context("doing something")
                .context("more context")?;
            Ok(())
        };

        let result = context_error();
        assert_eq!(
            result.unwrap_err().to_string(),
            bad_request!(
                "more context: doing something: code: bad_request, description: invalid input"
            )
            .to_string()
        );
    }

    // Test that error details are returned as json.
    #[test]
    fn r9k_context() {
        let result = Err::<(), Error>(server_error!("server error")).context("request context");
        let err: Error = result.unwrap_err().into();

        assert_eq!(
            err.to_string(),
            "code: server_error, description: request context: code: server_error, description: server error"
        );
    }

    #[test]
    fn anyhow_context() {
        let result = Err::<(), anyhow::Error>(anyhow!("one-off error")).context("error context");
        let err: Error = result.unwrap_err().into();

        assert_eq!(
            err.to_string(),
            "code: server_error, description: error context: one-off error"
        );
    }

    #[test]
    fn serde_context() {
        let result: Result<Value, anyhow::Error> =
            serde_json::from_str(r#"{"foo": "bar""#).context("error context");
        let err: Error = result.unwrap_err().into();

        assert_eq!(
            err.to_string(),
            "code: server_error, description: error context: EOF while parsing an object at line 1 column 13"
        );
    }

    #[test]
    fn json_error_derives_status_from_code() {
        let err = Error::json("422", serde_json::json!({"error": "validation_failed"}));

        assert_eq!(err.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(err.code(), "422");
        assert_eq!(err.to_string(), "code: 422");
    }

    #[test]
    fn json_error_invalid_code() {
        let err = Error::json("not_a_number", serde_json::json!({"error": "oops"}));

        assert_eq!(err.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    // The externally tagged wire shape predates the struct representation;
    // stored client fixtures depend on it.
    #[test]
    fn wire_format_stable() {
        let body = serde_json::json!({"field": "email", "reason": "invalid"});
        let err = Error::json("400", body.clone());
        assert_eq!(
            serde_json::to_value(&err).expect("serialize"),
            serde_json::json!({"Json": {"code": "400", "body": body}}),
        );

        let err = bad_request!("invalid input");
        assert_eq!(
            serde_json::to_value(&err).expect("serialize"),
            serde_json::json!({
                "BadRequest": {"code": "bad_request", "description": "invalid input"}
            }),
        );

        let deserialized: Error =
            serde_json::from_value(serde_json::json!({"Json": {"code": "400", "body": body}}))
                .expect("deserialize");
        assert_eq!(deserialized.json_body(), Some(body));
        assert_eq!(deserialized.status(), StatusCode::BAD_REQUEST);
    }
}
