//! Typed guest-API seam: a path-parameter GET and a JSON POST carrying a
//! header prove typed input extraction and transport-neutral invocation
//! metadata through the real WASI request/response boundary.

use anyhow::{Context as _, Result};
use bytes::Bytes;
use omnia::wasmtime_wasi::ResourceTable;
use omnia::{Backend as _, HasHttp, Runtime};
use omnia_testkit::{http, single_guest};
use omnia_wasi_http::{HttpDefault, WasiHttp, WasiHttpCtxView};

use crate::fixture;

/// The `examples/guest-api` backend bundle.
#[derive(Clone)]
struct Bundle {
    http: HttpDefault,
}

impl HasHttp for Bundle {
    fn http_view<'a>(&'a mut self, table: &'a mut ResourceTable) -> WasiHttpCtxView<'a> {
        self.http.as_view(table)
    }
}

async fn runtime() -> Result<Runtime<Bundle>> {
    let bundle = Bundle {
        http: HttpDefault::connect().await.context("connecting http")?,
    };

    let guest = single_guest("guest_api_wasm.wasm", bundle).await?;
    guest.host::<WasiHttp>()?.into_runtime()
}

#[test]
fn routes() -> Result<()> {
    fixture::RT.block_on(async {
        let runtime = runtime().await?;

        let response = http::get(&runtime, "/greet/omnia").await?;
        assert!(response.status().is_success(), "explicit route handles the request");

        let body: serde_json::Value = serde_json::from_slice(response.body())?;
        assert_eq!(body["message"], "Hello, omnia!", "path parameter reached operation input");
        assert_eq!(body["owner"], "examples", "invoker owner reached operation context");

        // `::http` disambiguates the crate from the imported testkit module.
        let request = ::http::Request::post("http://localhost/greet")
            .header(::http::header::HOST, "localhost")
            .header(::http::header::CONTENT_TYPE, "application/json")
            .header("x-request-id", "42")
            .body(Bytes::from_static(br#"{"name":"post"}"#))
            .context("building POST request")?;
        let response = http::handle(&runtime, request).await?;
        assert!(response.status().is_success(), "explicit POST route handles the request");

        let body: serde_json::Value = serde_json::from_slice(response.body())?;
        assert_eq!(body["message"], "Hello, post!", "JSON body reached operation input");
        assert_eq!(body["request_id"], "42", "header reached invocation metadata");

        Ok(())
    })
}
