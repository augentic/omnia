use std::fmt::Debug;
use std::ops::Deref;

use http::{HeaderMap, StatusCode};

use crate::api::Body;

/// Top-level response data structure common to all handlers.
#[derive(Debug)]
pub struct Reply<B>
where
    B: Body,
{
    /// HTTP status code.
    pub status: StatusCode,

    /// HTTP headers, if any.
    pub headers: HeaderMap,

    /// Response body.
    pub body: B,
}

impl<B: Body> Reply<B> {
    /// Create a success response
    #[must_use]
    pub fn ok(body: B) -> Self {
        Self {
            status: StatusCode::OK,
            headers: HeaderMap::new(),
            body,
        }
    }

    /// Create a created response (201)
    #[must_use]
    pub fn created(body: B) -> Self {
        Self {
            status: StatusCode::CREATED,
            headers: HeaderMap::new(),
            body,
        }
    }

    /// Create an accepted response (202)
    #[must_use]
    pub fn accepted(body: B) -> Self {
        Self {
            status: StatusCode::ACCEPTED,
            headers: HeaderMap::new(),
            body,
        }
    }

    /// Check if response is successful (2xx)
    #[must_use]
    pub fn is_success(&self) -> bool {
        self.status.is_success()
    }

    /// Create a success response with a specific status code.
    #[must_use]
    pub const fn status(mut self, status: StatusCode) -> Self {
        self.status = status;
        self
    }

    /// Add headers to the response.
    #[must_use]
    pub fn headers(mut self, headers: HeaderMap) -> Self {
        self.headers = headers;
        self
    }
}

impl<B: Body> From<B> for Reply<B> {
    fn from(body: B) -> Self {
        Self {
            status: StatusCode::OK,
            headers: HeaderMap::new(),
            body,
        }
    }
}

impl<B: Body> Deref for Reply<B> {
    type Target = B;

    fn deref(&self) -> &Self::Target {
        &self.body
    }
}

// /// Implemented by the `Reply::body` to convert itself into a body compatible with
// /// `[IntoResponse]`.
// pub trait IntoBody: Body {
//     /// Convert into a body + content type.
//     ///
//     /// # Errors
//     ///
//     /// Returns an error if the body cannot be encoded (for example, if JSON
//     /// serialization fails).
//     fn into_body(self) -> anyhow::Result<Vec<u8>>;
// }

// impl<T> IntoResponse for Reply<T>
// where
//     T: IntoBody,
// {
//     fn into_response(self) -> Response {
//         let body = match self.body.into_body() {
//             Ok(v) => v,
//             Err(e) => {
//                 return (StatusCode::INTERNAL_SERVER_ERROR, format!("body encoding error: {e}"))
//                     .into_response();
//             }
//         };

//         let mut hm = self.headers;
//         if !hm.contains_key(header::CONTENT_TYPE) {
//             hm.insert(header::CONTENT_TYPE, HeaderValue::from_static("text/plain; charset=utf-8"));
//         }

//         let status = self.status;
//         (status, hm, body).into_response()
//     }
// }
