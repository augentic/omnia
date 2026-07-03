//! Errors

use http::StatusCode;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Result type used across the crate.
pub type Result<T> = anyhow::Result<T, Error>;

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

    /// A domain-controlled error carrying a JSON body for the error response.
    #[error("code: {code}")]
    Json {
        /// The error code.
        code: String,
        /// JSON body rendered in the error response.
        body: serde_json::Value,
    },
}

impl Error {
    /// Returns the HTTP status code associated with the variant.
    #[must_use]
    pub fn status(&self) -> StatusCode {
        match self {
            Self::BadRequest { .. } => StatusCode::BAD_REQUEST,
            Self::NotFound { .. } => StatusCode::NOT_FOUND,
            Self::ServerError { .. } => StatusCode::INTERNAL_SERVER_ERROR,
            Self::BadGateway { .. } => StatusCode::BAD_GATEWAY,
            Self::Json { code, .. } => code
                .parse::<u16>()
                .ok()
                .and_then(|n| StatusCode::from_u16(n).ok())
                .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
        }
    }

    /// Returns the error code for the variant.
    #[must_use]
    pub fn code(&self) -> String {
        match self {
            Self::BadRequest { code, .. }
            | Self::NotFound { code, .. }
            | Self::ServerError { code, .. }
            | Self::BadGateway { code, .. }
            | Self::Json { code, .. } => code.clone(),
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
            Self::Json { code, .. } => code.clone(),
        }
    }

    /// Returns the JSON body if this is a `Json` variant.
    #[must_use]
    pub fn json_body(&self) -> Option<serde_json::Value> {
        match self {
            Self::Json { body, .. } => Some(body.clone()),
            _ => None,
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
                Self::Json { code, body } => Self::Json {
                    code: code.clone(),
                    body: body.clone(),
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

/// Create a new `BadRequest` error.
#[macro_export]
macro_rules! bad_request {
    ($fmt:expr, $($arg:tt)*) => {
        $crate::Error::BadRequest { code: "bad_request".to_string(), description: format!($fmt, $($arg)*) }
    };
    ($desc:expr $(,)?) => {
        $crate::Error::BadRequest { code: "bad_request".to_string(), description: format!($desc) }
    };
}

/// Create a new `ServerError` error.
#[macro_export]
macro_rules! server_error {
    ($fmt:expr, $($arg:tt)*) => {
        $crate::Error::ServerError { code: "server_error".to_string(), description: format!($fmt, $($arg)*) }
    };
     ($err:expr $(,)?) => {
        $crate::Error::ServerError { code: "server_error".to_string(), description: format!($err) }
    };
}

/// Create a new `BadGateway` error.
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
        let err = Error::Json {
            code: "422".to_string(),
            body: serde_json::json!({"error": "validation_failed"}),
        };

        assert_eq!(err.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(err.code(), "422");
        assert_eq!(err.to_string(), "code: 422");
    }

    #[test]
    fn json_error_invalid_code_falls_back_to_500() {
        let err = Error::Json {
            code: "not_a_number".to_string(),
            body: serde_json::json!({"error": "oops"}),
        };

        assert_eq!(err.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[test]
    fn json_error_body_round_trips() {
        let body = serde_json::json!({"field": "email", "reason": "invalid"});
        let err = Error::Json {
            code: "400".to_string(),
            body: body.clone(),
        };

        let json = serde_json::to_string(&err).expect("serialize");
        let deserialized: Error = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(deserialized.json_body(), Some(body));
        assert_eq!(deserialized.status(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn json_error_custom_body() {
        let custom_body = serde_json::json!({"message": "oops", "field": "id"});
        let err = Error::Json {
            code: "409".to_string(),
            body: custom_body.clone(),
        };

        assert_eq!(err.json_body(), Some(custom_body));
        assert_eq!(err.status(), StatusCode::CONFLICT);
    }
}
