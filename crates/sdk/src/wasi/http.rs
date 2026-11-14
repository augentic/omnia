//! [`HttpClient`] over `wasi:http`, and the guest's `wasi:http` export.

use bytes::Bytes;
use http::{Request, Response};

use crate::{Error, HttpClient, Result, WasiProvider};

impl HttpClient for WasiProvider {
    async fn fetch(&self, request: Request<Bytes>) -> Result<Response<Bytes>> {
        let target = format!("{} {}", request.method(), request.uri());
        omnia_wasi_http::handle(request.map(http_body_util::Full::new))
            .await
            .map_err(|err| Error::bad_gateway("unavailable", format!("sending {target}: {err:#}")))
    }
}

/// Serve one `wasi:http` request through `router`.
///
/// The request runs inside the guest telemetry scope and the `handler` span
/// [`trace`](crate::api::http::trace) opens, so a router served here does not
/// layer `trace` itself. [`http::export!`](crate::api::http::export!) expands
/// to this call, and a hand-written `wasi:http` export calls it directly.
///
/// # Errors
///
/// Returns the WASI error code when the request or the router's response
/// does not convert.
#[cfg(feature = "http")]
#[omnia_wasi_otel::instrument(name = "http_guest_handle")]
pub async fn serve(
    router: impl Into<axum::Router>, request: wasip3::http::types::Request,
) -> Result<wasip3::http::types::Response, wasip3::http::types::ErrorCode> {
    let router = router.into().layer(axum::middleware::from_fn(crate::api::http::trace));
    omnia_wasi_http::serve(router, request).await
}
