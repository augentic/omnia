//! Seam test for the explicit typed guest API through the real WASI
//! request/response boundary.
//!
//! A path-parameter GET and a JSON POST carrying a header prove typed input
//! extraction and transport-neutral invocation metadata.
//!
//! The guest is built automatically on first [`find_guest`] call; the test
//! skips locally when it is absent and fails under CI so the pipeline never
//! passes vacuously.

#![cfg(not(target_arch = "wasm32"))]

use anyhow::{Context as _, Result};
use bytes::Bytes;
use omnia::wasmtime_wasi::ResourceTable;
use omnia::{Backend as _, HasHttp, Runtime};
use omnia_testkit::{http, single_guest};
use omnia_wasi_http::{HttpDefault, WasiHttp, WasiHttpCtxView};

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

async fn runtime() -> Result<Option<Runtime<Bundle>>> {
    let bundle = Bundle {
        http: HttpDefault::connect().await.context("connecting http")?,
    };

    let Some(guest) = single_guest("guest_api_wasm.wasm", bundle).await? else {
        return Ok(None);
    };
    Ok(Some(guest.host::<WasiHttp>()?.into_runtime()?))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn routes() -> Result<()> {
    let Some(runtime) = runtime().await? else {
        return Ok(());
    };

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
}
