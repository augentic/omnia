//! Outbound HTTP capability.

use std::future::Future;

use bytes::Bytes;
use http::{Request, Response};
use serde::de::DeserializeOwned;

use crate::capability::Result;

/// Send outbound HTTP requests.
///
/// Any status is a response, not an error. A request the upstream did not
/// answer fails with
/// [`ErrorKind::Unavailable`](crate::capability::ErrorKind::Unavailable).
/// JSON reads of an upstream reply are [`HttpClientExt`].
#[omnia_guest_macros::capability]
pub trait HttpClient: Send + Sync {
    /// Send `request` and return the upstream's response.
    fn fetch(
        &self, request: Request<Bytes>,
    ) -> impl Future<Output = Result<Response<Bytes>>> + Send;
}

/// JSON reads over any [`HttpClient`].
///
/// Blanket-implemented for every [`HttpClient`]; never implemented by hand.
pub trait HttpClientExt: HttpClient {
    /// Send `request`, require a 2xx status, and deserialize the body as JSON.
    ///
    /// # Errors
    ///
    /// - [`crate::Error::BadGateway`] when the upstream does not answer,
    ///   returns a non-2xx status, or returns a body that is not the JSON of
    ///   an `O`.
    /// - [`crate::Error::ServerError`] when the request fails for any other
    ///   reason.
    fn fetch_json<O: DeserializeOwned>(
        &self, request: Request<Bytes>,
    ) -> impl Future<Output = crate::Result<O>> + Send {
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

impl<P: HttpClient + ?Sized> HttpClientExt for P {}

#[cfg(all(target_arch = "wasm32", target_os = "wasi"))]
impl HttpClient for crate::WasiProvider {
    async fn fetch(&self, request: Request<Bytes>) -> Result<Response<Bytes>> {
        use crate::capability::{Error, ErrorKind};

        let target = format!("{} {}", request.method(), request.uri());
        omnia_wasi_http::handle(request.map(http_body_util::Full::new)).await.map_err(|err| {
            Error::new(ErrorKind::Unavailable, format!("sending {target}")).with_source(err)
        })
    }
}
