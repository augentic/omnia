//! [`HttpClient`] over `wasi:http`.

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
