//! Outbound HTTP capability.

use std::any::Any;
use std::error::Error;
use std::future::Future;

use anyhow::Result;
use bytes::Bytes;
use http::{Request, Response};
use http_body::Body;

/// Fetches data from an outbound HTTP source.
pub trait HttpRequest: Send + Sync {
    /// Make outbound HTTP request.
    #[cfg(not(target_arch = "wasm32"))]
    fn fetch<T>(&self, request: Request<T>) -> impl Future<Output = Result<Response<Bytes>>> + Send
    where
        T: Body + Any + Send,
        T::Data: Into<Vec<u8>>,
        T::Error: Into<Box<dyn Error + Send + Sync + 'static>>;

    /// Make outbound HTTP request.
    #[cfg(target_arch = "wasm32")]
    fn fetch<T>(&self, request: Request<T>) -> impl Future<Output = Result<Response<Bytes>>> + Send
    where
        T: Body + Any + Send,
        T::Data: Into<Vec<u8>>,
        T::Error: Into<Box<dyn Error + Send + Sync + 'static>>,
    {
        async move { omnia_wasi_http::handle(request).await }
    }
}
