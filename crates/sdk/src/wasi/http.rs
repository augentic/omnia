//! [`HttpClient`] over `wasi:http`.

use bytes::Bytes;
use http::{Request, Response};

use crate::{CapabilityError, ErrorKind, HttpClient, WasiProvider};

impl HttpClient for WasiProvider {
    async fn fetch(&self, request: Request<Bytes>) -> Result<Response<Bytes>, CapabilityError> {
        let target = format!("{} {}", request.method(), request.uri());
        omnia_wasi_http::handle(request.map(http_body_util::Full::new)).await.map_err(|err| {
            CapabilityError::new(ErrorKind::Unavailable, format!("sending {target}"))
                .with_source(err)
        })
    }
}
