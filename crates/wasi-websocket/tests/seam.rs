//! Seam test for `wasi:websocket`: drive the `websocket` example guest over the
//! real `wasi:http` boundary and confirm it can traverse the websocket boundary.
//!
//! The guest's `POST /` connects a websocket client and sends an event. With no
//! external socket connected the send has no externally observable effect, so
//! the contract this asserts is that the guest's `connect` + `send` cross the
//! `wasi:websocket` boundary and return without trapping (a `200` carrying the
//! guest's `event sent` acknowledgement).
//!
//! The guest is built automatically on first [`find_guest`] call; the test skips locally when
//! it is absent and fails under CI so the pipeline never passes vacuously.

#![cfg(not(target_arch = "wasm32"))]

use std::sync::Arc;

use anyhow::{Context as _, Result};
use omnia::wasmtime_wasi::ResourceTable;
use omnia::{Backend, DeploymentBuilder, HasHttp, MountRegistry, Runtime, StoreCtx};
use omnia_testkit::{find_guest, http};
use omnia_wasi_http::{HttpDefault, WasiHttp, WasiHttpCtxView};
use omnia_wasi_otel::{HasOtel, OtelDefault, WasiOtel, WasiOtelCtx};
use omnia_wasi_websocket::{HasWebSocket, WasiWebSocket, WasiWebSocketCtx, WebSocketDefault};

/// The `examples/websocket` backend bundle: `wasi:http` + `wasi:otel` +
/// `wasi:websocket`.
#[derive(Clone)]
struct Bundle {
    http: HttpDefault,
    otel: OtelDefault,
    websocket: WebSocketDefault,
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

impl HasWebSocket for Bundle {
    fn websocket_ctx(&mut self) -> &mut dyn WasiWebSocketCtx {
        &mut self.websocket
    }
}

async fn runtime() -> Result<Option<Runtime<Bundle>>> {
    let Some(wasm) = find_guest("websocket_wasm.wasm") else {
        return Ok(None);
    };

    let bundle = Bundle {
        http: HttpDefault::connect().await.context("connecting http")?,
        otel: OtelDefault::connect().await.context("connecting otel")?,
        websocket: <WebSocketDefault as Backend>::connect()
            .await
            .context("connecting websocket")?,
    };

    let mut deployment =
        DeploymentBuilder::new().wasm(wasm).build::<StoreCtx<Bundle>>().await.context("build")?;
    deployment.host::<WasiHttp, Bundle>().context("link http")?;
    deployment.host::<WasiOtel, Bundle>().context("link otel")?;
    deployment.host::<WasiWebSocket, Bundle>().context("link websocket")?;
    let registry = deployment.into_registry().context("assemble registry")?;

    Ok(Some(Runtime::from_parts(
        Arc::new(registry),
        Vec::new(),
        Arc::new(MountRegistry::default()),
        bundle,
    )))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn send_crosses_boundary() -> Result<()> {
    let Some(runtime) = runtime().await? else {
        return Ok(());
    };

    let response = http::post(&runtime, "/", "hello sockets").await?;
    assert!(response.status().is_success(), "guest connects and sends across the ws boundary");

    let body: serde_json::Value = serde_json::from_slice(response.body())?;
    assert_eq!(
        body,
        serde_json::json!({ "message": "event sent" }),
        "guest acknowledges the send it drove through the host"
    );

    Ok(())
}
