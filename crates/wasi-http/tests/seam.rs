//! Seam test for `wasi:http`: drive the `http` example guest through the real
//! request/response boundary this crate implements.
//!
//! The guest is an axum echo (`POST /` → `{"message", "request"}`), so a `200`
//! whose `request` mirrors the body proves a request crossed into the guest and
//! a response came back — exercising `Request::from_http` / `Response::into_http`
//! and the trigger router without a TCP socket.
//!
//! The guest is built by `cargo make build-guests`; the test skips locally when
//! it is absent and fails under CI so the pipeline never passes vacuously.

#![cfg(not(target_arch = "wasm32"))]

use std::sync::Arc;

use anyhow::{Context as _, Result};
use omnia::wasmtime_wasi::ResourceTable;
use omnia::{Backend as _, DeploymentBuilder, HasHttp, MountRegistry, Runtime, StoreCtx};
use omnia_testkit::{find_guest, http};
use omnia_wasi_http::{HttpDefault, WasiHttp, WasiHttpCtxView};
use omnia_wasi_otel::{HasOtel, OtelDefault, WasiOtel, WasiOtelCtx};

/// The `examples/http` backend bundle: `wasi:http` + `wasi:otel` (the guest
/// instruments its handler).
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
    let Some(wasm) = find_guest("http_wasm.wasm", "cargo make build-guests") else {
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
async fn echo() -> Result<()> {
    let Some(runtime) = runtime().await? else {
        return Ok(());
    };

    let response = http::post_json(&runtime, "/", r#"{"ping":"pong"}"#).await?;
    assert!(response.status().is_success(), "guest handles the request across the boundary");

    let body: serde_json::Value = serde_json::from_slice(response.body())?;
    assert_eq!(
        body["request"],
        serde_json::json!({ "ping": "pong" }),
        "the guest echoes the request body back across the boundary"
    );

    Ok(())
}
