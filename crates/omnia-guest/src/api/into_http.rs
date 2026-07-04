use axum::response::{IntoResponse, Response};
use http::header::CONTENT_TYPE;
use http::{HeaderValue, StatusCode};

use crate::api::Body;
use crate::api::reply::Reply;

/// Result type for HTTP requests; `T` and `E` are expected to implement
/// [`IntoResponse`].
pub type HttpResult<T, E = HttpError> = Result<T, E>;

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

/// Error type for HTTP requests.
pub struct HttpError {
    status: StatusCode,
    error: String,
    content_type: Option<HeaderValue>,
}

impl From<crate::Error> for HttpError {
    fn from(e: crate::Error) -> Self {
        if let Some(body) = e.json_body() {
            return Self {
                status: e.status(),
                error: serde_json::to_string(&body).unwrap_or_else(|_| e.to_string()),
                content_type: Some(HeaderValue::from_static("application/json")),
            };
        }

        Self {
            status: e.status(),
            error: e.to_string(),
            content_type: None,
        }
    }
}

impl From<anyhow::Error> for HttpError {
    fn from(e: anyhow::Error) -> Self {
        if e.downcast_ref::<crate::Error>().is_some() {
            let sdk_err: crate::Error = e.into();
            return Self::from(sdk_err);
        }

        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            error: format!("{e}, caused by: {}", e.root_cause()),
            content_type: None,
        }
    }
}

impl IntoResponse for HttpError {
    fn into_response(self) -> axum::response::Response {
        match self.content_type {
            Some(content_type) => {
                let mut headers = http::HeaderMap::new();
                headers.insert(CONTENT_TYPE, content_type);
                (self.status, headers, self.error).into_response()
            }
            None => (self.status, self.error).into_response(),
        }
    }
}

#[cfg(test)]
mod tests {
    use axum::response::IntoResponse;
    use http::StatusCode;
    use http::header::CONTENT_TYPE;
    use http_body_util::BodyExt;

    use super::HttpError;

    async fn collect_body(response: axum::response::Response) -> Vec<u8> {
        response.into_body().collect().await.expect("collect body").to_bytes().to_vec()
    }

    #[tokio::test]
    async fn json_content_type() {
        let body =
            serde_json::json!({"error": "invalid_request", "error_description": "missing field"});
        let err = crate::Error::Json {
            code: "400".to_string(),
            body: body.clone(),
        };

        let http_err = HttpError::from(err);
        let response = http_err.into_response();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(response.headers().get(CONTENT_TYPE).expect("content-type"), "application/json");

        let response_body = collect_body(response).await;
        let parsed: serde_json::Value = serde_json::from_slice(&response_body).expect("parse json");
        assert_eq!(parsed, body);
    }

    #[tokio::test]
    async fn wrapped_json_body() {
        use anyhow::Context;

        let body = serde_json::json!({"error": "invalid_request", "error_description": "bad"});
        let err = crate::Error::Json {
            code: "422".to_string(),
            body: body.clone(),
        };

        let anyhow_err: anyhow::Error = Err::<(), _>(err).context("extra context").unwrap_err();
        let http_err = HttpError::from(anyhow_err);
        let response = http_err.into_response();

        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(response.headers().get(CONTENT_TYPE).expect("content-type"), "application/json");

        let response_body = collect_body(response).await;
        let parsed: serde_json::Value = serde_json::from_slice(&response_body).expect("parse json");
        assert_eq!(parsed, body);
    }
}
