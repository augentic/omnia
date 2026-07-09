//! Seam test for the `guest!` macro: drive the `guest-api` example guest —
//! whose WASI export, router, and handler glue are all macro-generated —
//! through the real request/response boundary.
//!
//! A path-parameter GET and a JSON POST carrying a header prove that typed
//! extraction reaches `Handler::from_input` and that inbound headers arrive
//! in `Context::headers`.
//!
//! The guest is built automatically on first [`find_guest`] call; the test
//! skips locally when it is absent and fails under CI so the pipeline never
//! passes vacuously.

#![cfg(not(target_arch = "wasm32"))]

use std::sync::Arc;

use anyhow::{Context as _, Result};
use bytes::Bytes;
use omnia::wasmtime_wasi::ResourceTable;
use omnia::{Backend as _, DeploymentBuilder, HasHttp, MountRegistry, Runtime, StoreCtx};
use omnia_testkit::{find_guest, http};
use omnia_wasi_http::{HttpDefault, WasiHttp, WasiHttpCtxView};
use omnia_wasi_otel::{HasOtel, OtelDefault, WasiOtel, WasiOtelCtx};

/// The `examples/guest-api` backend bundle: `wasi:http` + `wasi:otel` (the
/// macro instruments generated handlers).
#[derive(Clone)]
struct Bundle {
    http: HttpDefault,
    otel: OtelDefault,
}

impl HasHttp for Bundle {
    fn http_view<'a>(&'a mut self, table: &'a mut ResourceTable) -> WasiHttpCtxView<'a> {
        self.http.as_view(table)
    }
}

impl HasOtel for Bundle {
    fn otel_ctx(&mut self) -> &mut dyn WasiOtelCtx {
        &mut self.otel
    }
}

async fn runtime() -> Result<Option<Runtime<Bundle>>> {
    let Some(wasm) = find_guest("guest_api_wasm.wasm") else {
        return Ok(None);
    };

    let bundle = Bundle {
        http: HttpDefault::connect().await.context("connecting http")?,
        otel: OtelDefault::connect().await.context("connecting otel")?,
    };

    let mut deployment =
        DeploymentBuilder::new().wasm(wasm).build::<StoreCtx<Bundle>>().await.context("build")?;
    deployment.host::<WasiHttp, Bundle>().context("link http")?;
    deployment.host::<WasiOtel, Bundle>().context("link otel")?;
    let registry = deployment.into_registry().context("assemble registry")?;

    Ok(Some(Runtime::from_parts(
        Arc::new(registry),
        Vec::new(),
        Arc::new(MountRegistry::default()),
        bundle,
    )))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn path_parameter_get() -> Result<()> {
    let Some(runtime) = runtime().await? else {
        return Ok(());
    };

    let response = http::get(&runtime, "/greet/omnia").await?;
    assert!(response.status().is_success(), "macro-generated route handles the request");

    let body: serde_json::Value = serde_json::from_slice(response.body())?;
    assert_eq!(body["message"], "Hello, omnia!", "path parameter reached Handler::from_input");
    assert_eq!(body["owner"], "examples", "the macro's owner arrives in Context");

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn json_post_with_headers() -> Result<()> {
    let Some(runtime) = runtime().await? else {
        return Ok(());
    };

    // `::http` disambiguates the crate from the imported testkit module.
    let request = ::http::Request::post("http://localhost/greet")
        .header(::http::header::HOST, "localhost")
        .header(::http::header::CONTENT_TYPE, "application/json")
        .header("x-request-id", "42")
        .body(Bytes::from_static(br#"{"name":"post"}"#))
        .context("building POST request")?;
    let response = http::handle(&runtime, request).await?;
    assert!(response.status().is_success(), "macro-generated POST route handles the request");

    let body: serde_json::Value = serde_json::from_slice(response.body())?;
    assert_eq!(body["message"], "Hello, post!", "JSON body reached Handler::from_input");
    assert_eq!(body["request_id"], "42", "inbound headers arrive in Context::headers");

    Ok(())
}
