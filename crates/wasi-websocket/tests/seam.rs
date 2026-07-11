//! Seam test for `wasi:websocket`: drive the `websocket` example guest over the
//! real `wasi:http` boundary and observe its event arrive at a connected
//! WebSocket client.
//!
//! The guest's `POST /` connects a websocket client and sends an event. The
//! default backend forwards sends to externally connected peers, so a real
//! tungstenite client connected to the backend's server receiving the payload
//! proves the guest's `connect` + `send` crossed the `wasi:websocket` boundary
//! into the host — rather than merely returning `200`.
//!
//! The guest is built automatically on first [`find_guest`] call; the test skips locally when
//! it is absent and fails under CI so the pipeline never passes vacuously.

#![cfg(not(target_arch = "wasm32"))]

use std::net::TcpListener;
use std::time::Duration;

use anyhow::{Context as _, Result};
use futures_util::StreamExt as _;
use omnia::wasmtime_wasi::ResourceTable;
use omnia::{Backend, HasHttp, Runtime};
use omnia_testkit::{http, single_guest};
use omnia_wasi_http::{HttpDefault, WasiHttp, WasiHttpCtxView};
use omnia_wasi_otel::{HasOtel, OtelDefault, WasiOtel, WasiOtelCtx};
use omnia_wasi_websocket::{
    ConnectOptions, HasWebSocket, WasiWebSocket, WasiWebSocketCtx, WebSocketDefault,
};
use tokio_tungstenite::tungstenite::Message;

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

/// Reserve a free localhost port for the backend's WebSocket server.
fn free_port() -> Result<u16> {
    let listener = TcpListener::bind("127.0.0.1:0").context("reserving a port")?;
    Ok(listener.local_addr()?.port())
}

/// Build the runtime with the backend's server bound to `socket_addr`.
async fn runtime(socket_addr: String) -> Result<Option<Runtime<Bundle>>> {
    let bundle = Bundle {
        http: HttpDefault::connect().await.context("connecting http")?,
        otel: OtelDefault::connect().await.context("connecting otel")?,
        websocket: WebSocketDefault::connect_with(ConnectOptions { socket_addr })
            .await
            .context("connecting websocket")?,
    };

    let Some(guest) = single_guest("websocket_wasm.wasm", bundle).await? else {
        return Ok(None);
    };
    Ok(Some(guest.host::<WasiHttp>()?.host::<WasiOtel>()?.host::<WasiWebSocket>()?.into_runtime()?))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn send_reaches_connected_peer() -> Result<()> {
    let port = free_port()?;
    let Some(runtime) = runtime(format!("127.0.0.1:{port}")).await? else {
        return Ok(());
    };

    // The backend's server starts on a spawned task; retry until it accepts.
    let url = format!("ws://127.0.0.1:{port}");
    let mut peer = None;
    for _ in 0..50 {
        match tokio_tungstenite::connect_async(&url).await {
            Ok((stream, _)) => {
                peer = Some(stream);
                break;
            }
            Err(_) => tokio::time::sleep(Duration::from_millis(100)).await,
        }
    }
    let mut peer = peer.context("websocket server did not accept a connection")?;

    // The guest's send only reaches peers registered before it fires; the
    // handshake and registration race, so retry the request until delivery.
    let mut delivered = None;
    for _ in 0..10 {
        let response = http::post(&runtime, "/", "hello sockets").await?;
        assert!(response.status().is_success(), "guest connects and sends across the ws boundary");
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(response.body())?,
            serde_json::json!({ "message": "event sent" }),
            "guest acknowledges the send it drove through the host"
        );

        match tokio::time::timeout(Duration::from_secs(1), peer.next()).await {
            Ok(message) => {
                delivered = Some(message.context("connection closed without a message")??);
                break;
            }
            Err(_elapsed) => {}
        }
    }

    let message = delivered.context("guest event never reached the connected peer")?;
    assert_eq!(
        message,
        Message::Binary(b"hello sockets".as_slice().into()),
        "the guest's payload reached the external peer intact"
    );

    Ok(())
}
