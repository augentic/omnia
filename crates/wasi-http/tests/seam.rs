//! Seam test for `wasi:http`: drive the `http` example guest through the real
//! request/response boundary this crate implements.
//!
//! The guest is an axum echo (`POST /` → `{"message", "request"}`), so a `200`
//! whose `request` mirrors the body proves a request crossed into the guest and
//! a response came back — exercising `Request::from_http` / `Response::into_http`
//! and the trigger router without a TCP socket.
//!
//! The guest is built automatically on first [`find_guest`] call; the test skips locally when
//! it is absent and fails under CI so the pipeline never passes vacuously.

#![cfg(not(target_arch = "wasm32"))]

use anyhow::{Context as _, Result};
use omnia::wasmtime_wasi::ResourceTable;
use omnia::{Backend as _, HasHttp, Runtime};
use omnia_testkit::{http, single_guest};
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
    let bundle = Bundle {
        http: HttpDefault::connect().await.context("connecting http")?,
        otel: OtelDefault::connect().await.context("connecting otel")?,
    };

    let Some(guest) = single_guest("http_wasm.wasm", bundle).await? else {
        return Ok(None);
    };
    Ok(Some(guest.host::<WasiHttp>()?.host::<WasiOtel>()?.into_runtime()?))
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
