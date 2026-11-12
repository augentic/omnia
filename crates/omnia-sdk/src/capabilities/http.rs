//! Outbound HTTP capability.

use std::any::Any;
use std::error::Error;
use std::future::Future;

use anyhow::Result;
use bytes::Bytes;
use http::{Request, Response};
use http_body::Body;
use serde::de::DeserializeOwned;

/// Fetches data from an outbound HTTP source.
///
/// JSON reads of an upstream reply are [`HttpRequestExt`].
#[omnia_guest_macros::capability]
pub trait HttpRequest: Send + Sync {
    /// Make outbound HTTP request.
    fn fetch<T>(&self, request: Request<T>) -> impl Future<Output = Result<Response<Bytes>>> + Send
    where
        T: Body + Any + Send,
        T::Data: Into<Vec<u8>>,
        T::Error: Into<Box<dyn Error + Send + Sync + 'static>>;
}

/// JSON reads over any [`HttpRequest`].
///
/// Blanket-implemented for every [`HttpRequest`]; never implemented by hand.
pub trait HttpRequestExt: HttpRequest {
    /// Fetch an outbound HTTP request, require a 2xx status (`502 Bad Gateway`
    /// otherwise), and deserialize the response body as JSON.
    ///
    /// # Errors
    ///
    /// Returns [`crate::Error::ServerError`] when the transport fails and
    /// [`crate::Error::BadGateway`] when the upstream returns a non-success
    /// status or an unreadable JSON body.
    fn fetch_json<T, O>(&self, request: Request<T>) -> impl Future<Output = crate::Result<O>> + Send
    where
        T: Body + Any + Send,
        T::Data: Into<Vec<u8>>,
        T::Error: Into<Box<dyn Error + Send + Sync + 'static>>,
        O: DeserializeOwned,
    {
        async move {
            let response = self.fetch(request).await?;
            if !response.status().is_success() {
                return Err(crate::bad_gateway!("upstream returned {}", response.status()));
            }
            serde_json::from_slice(&response.into_body())
                .map_err(|error| crate::bad_gateway!("parsing upstream response: {error}"))
        }
    }
}

impl<P: HttpRequest + ?Sized> HttpRequestExt for P {}

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
