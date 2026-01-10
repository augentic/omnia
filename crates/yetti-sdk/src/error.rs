//! Errors

// use axum::response::{IntoResponse, Response};
use http::StatusCode;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Result type used across the crate.
pub type Result<T> = anyhow::Result<T, Error>;

/// Domain level error type returned by the adapter.
#[derive(Error, Debug, Clone, Serialize, Deserialize)]
pub enum Error {
    // --- Client errors ---
    /// Request payload is invalid or missing required fields.
    #[error("code: {code}, description: {description}")]
    BadRequest { code: String, description: String },

    /// Resource or data not found.
    #[error("code: {code}, description: {description}")]
    NotFound { code: String, description: String },

    // --- Server errors ---
    /// A non recoverable internal error occurred.
    #[error("code: {code}, description: {description}")]
    ServerError { code: String, description: String },

    /// An upstream dependency failed while fulfilling the request.
    #[error("code: {code}, description: {description}")]
    BadGateway { code: String, description: String },
}

impl Error {
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

impl From<anyhow::Error> for Error {
    fn from(err: anyhow::Error) -> Self {
        let chain = err.chain().map(ToString::to_string).collect::<Vec<_>>().join(": ");

        // if type is Error, return it with the newly added context
        if let Some(inner) = err.downcast_ref::<Self>() {
            tracing::debug!("Error: {err}, caused by: {inner}");

            return match inner {
                Self::BadRequest { code, .. } => Self::BadRequest {
                    code: code.clone(),
                    description: chain,
                },
                Self::NotFound { code, .. } => Self::NotFound {
                    code: code.clone(),
                    description: chain,
                },
                Self::ServerError { code, .. } => Self::ServerError {
                    code: code.clone(),
                    description: chain,
                },
                Self::BadGateway { code, .. } => Self::BadGateway {
                    code: code.clone(),
                    description: chain,
                },
            };
        }

        // otherwise, return an Internal error
        Self::ServerError {
            code: "server_error".to_string(),
            description: chain,
        }
    }
}

impl From<serde_json::Error> for Error {
    fn from(err: serde_json::Error) -> Self {
        Self::BadRequest {
            code: "serde_json".to_string(),
            description: err.to_string(),
        }
    }
}

#[macro_export]
macro_rules! bad_request {
    ($fmt:expr, $($arg:tt)*) => {
        $crate::Error::BadRequest { code: "bad_request".to_string(), description: format!($fmt, $($arg)*) }
    };
    ($desc:expr $(,)?) => {
        $crate::Error::BadRequest { code: "bad_request".to_string(), description: format!($desc) }
    };
}

#[macro_export]
macro_rules! server_error {
    ($fmt:expr, $($arg:tt)*) => {
        $crate::Error::ServerError { code: "server_error".to_string(), description: format!($fmt, $($arg)*) }
    };
     ($err:expr $(,)?) => {
        $crate::Error::ServerError { code: "server_error".to_string(), description: format!($err) }
    };
}

#[macro_export]
macro_rules! bad_gateway {
    ($fmt:expr, $($arg:tt)*) => {
        $crate::Error::BadGateway { code: "bad_gateway".to_string(), description: format!($fmt, $($arg)*) }
    };
     ($err:expr $(,)?) => {
        $crate::Error::BadGateway { code: "bad_gateway".to_string(), description: format!($err) }
    };
}

#[cfg(test)]
mod tests {
    use anyhow::{Context, Result, anyhow};
    use serde_json::Value;
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;
    use tracing_subscriber::{EnvFilter, Registry, fmt};

    use super::Error;

    #[test]
    fn error_display() {
        let err = bad_request!("invalid input");
        assert_eq!(format!("{err}",), "code: bad_request, description: invalid input");
    }

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
}
