//! Outbound HTTP capability.

use std::any::Any;
use std::error::Error;
use std::future::Future;

use anyhow::Result;
use bytes::Bytes;
use http::{Request, Response};
use http_body::Body;

/// Fetches data from an outbound HTTP source.
#[omnia_guest_macros::capability]
pub trait HttpRequest: Send + Sync {
    /// Make outbound HTTP request.
    fn fetch<T>(&self, request: Request<T>) -> impl Future<Output = Result<Response<Bytes>>> + Send
    where
        T: Body + Any + Send,
        T::Data: Into<Vec<u8>>,
        T::Error: Into<Box<dyn Error + Send + Sync + 'static>>;
}

#[cfg(all(target_arch = "wasm32", target_os = "wasi"))]
impl HttpRequest for crate::WasiProvider {
    async fn fetch<T>(&self, request: Request<T>) -> Result<Response<Bytes>>
    where
        T: Body + Any + Send,
        T::Data: Into<Vec<u8>>,
        T::Error: Into<Box<dyn Error + Send + Sync + 'static>>,
    {
        omnia_wasi_http::handle(request).await
    }
}
