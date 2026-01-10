use axum::response::{IntoResponse, Response};
use http::header::CONTENT_TYPE;
use http::{HeaderValue, StatusCode};

use crate::api::Body;
use crate::api::reply::Reply;

#[allow(type_alias_bounds)]
pub type HttpResult<T: IntoResponse, E: IntoResponse = HttpError> = Result<T, E>;

/// Implemented by the `Reply::body` to convert itself into a format compatible
/// with `[IntoResponse]`.
pub trait IntoBody: Body {
    /// Convert implementing type into an http-compatible body.
    ///
    /// # Errors
    ///
    /// Returns an error if the body cannot be encoded (for example, if JSON
    /// serialization fails).
    fn into_body(self) -> anyhow::Result<Vec<u8>>;
}

impl<T> IntoResponse for Reply<T>
where
    T: IntoBody,
{
    fn into_response(self) -> Response {
        let body = match self.body.into_body() {
            Ok(v) => v,
            Err(e) => {
                return (StatusCode::INTERNAL_SERVER_ERROR, format!("body encoding error: {e}"))
                    .into_response();
            }
        };

        let mut hm = self.headers;
        if !hm.contains_key(CONTENT_TYPE) {
            hm.insert(CONTENT_TYPE, HeaderValue::from_static("text/plain; charset=utf-8"));
        }

        let status = self.status;
        (status, hm, body).into_response()
    }
}

pub struct HttpError {
    status: StatusCode,
    error: String,
}

impl From<crate::Error> for HttpError {
    fn from(e: crate::Error) -> Self {
        Self {
            status: e.status(),
            error: e.to_string(),
        }
    }
}

impl From<anyhow::Error> for HttpError {
    fn from(e: anyhow::Error) -> Self {
        let error = format!("{e}, caused by: {}", e.root_cause());
        let status =
            e.downcast_ref().map_or(StatusCode::INTERNAL_SERVER_ERROR, crate::Error::status);
        Self { status, error }
    }
}

impl IntoResponse for HttpError {
    fn into_response(self) -> axum::response::Response {
        (self.status, self.error).into_response()
    }
}
