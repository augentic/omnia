//! [`HttpClient`] over `wasi:http`.

use bytes::Bytes;
use http::{Request, Response};

use crate::capability::{Error, ErrorKind, Result};
use crate::{HttpClient, WasiProvider};

impl HttpClient for WasiProvider {
    async fn fetch(&self, request: Request<Bytes>) -> Result<Response<Bytes>> {
        let target = format!("{} {}", request.method(), request.uri());
        omnia_wasi_http::handle(request.map(http_body_util::Full::new)).await.map_err(|err| {
            Error::new(ErrorKind::Unavailable, format!("sending {target}")).with_source(err)
        })
    }
}
