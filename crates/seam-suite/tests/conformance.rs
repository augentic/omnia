//! Scenarios driven through the conformance guest (`examples/conformance`).
//!
//! Everything here pins boundary behavior the guest alone cannot prove or
//! observe: the outbound `wasi:http` policy applied in the default backend's
//! reqwest hook (asserted at a wiremock origin), the `wasi:keyvalue` CAS legs
//! whose `cas-failed` error threads a fresh resource handle back across WIT,
//! and the trigger legs — a guest websocket send reaching a connected peer, a
//! peer message reaching the guest handler, and a guest publish landing on
//! the host broker.

use std::net::TcpListener;
use std::time::Duration;

use anyhow::{Context as _, Result, ensure};
use futures::{SinkExt as _, StreamExt as _};
use omnia::wasmtime_wasi::ResourceTable;
use omnia::{Backend, HasHttp, Runtime};
use omnia_testkit::{http, single_guest};
use omnia_wasi_http::{HttpDefault, WasiHttp, WasiHttpCtxView};
use omnia_wasi_keyvalue::{HasKeyValue, KeyValueDefault, WasiKeyValue, WasiKeyValueCtx};
use omnia_wasi_messaging::{HasMessaging, MessagingDefault, WasiMessaging, WasiMessagingCtx};
use omnia_wasi_websocket::{HasWebSocket, WasiWebSocket, WasiWebSocketCtx, WebSocketDefault};
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::Message;
use wiremock::matchers::{body_string, header, method};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// The backend bundle behind the conformance guest.
#[derive(Clone)]
struct Bundle {
    http: HttpDefault,
    keyvalue: KeyValueDefault,
    messaging: MessagingDefault,
    websocket: WebSocketDefault,
}

impl HasHttp for Bundle {
    fn http_view<'a>(&'a mut self, table: &'a mut ResourceTable) -> WasiHttpCtxView<'a> {
        self.http.as_view(table)
    }
}

impl HasKeyValue for Bundle {
    fn keyvalue_ctx(&mut self) -> &mut dyn WasiKeyValueCtx {
        &mut self.keyvalue
    }
}

impl HasMessaging for Bundle {
    fn messaging_ctx(&mut self) -> &mut dyn WasiMessagingCtx {
        &mut self.messaging
    }
}

impl HasWebSocket for Bundle {
    fn websocket_ctx(&mut self) -> &mut dyn WasiWebSocketCtx {
        &mut self.websocket
    }
}

/// The runtime plus probe handles onto the shared backends (clones share
/// state, so a probe observes the guest's effects) and the port the default
/// WebSocket backend's server listens on.
struct Fx {
    runtime: Runtime<Bundle>,
    keyvalue: KeyValueDefault,
    messaging: MessagingDefault,
    websocket_port: u16,
}

async fn fx() -> Result<Fx> {
    // Pre-bind the WebSocket listener so the port is known without a
    // drop-and-rebind race; the backend serves on it directly.
    let websocket_listener =
        TcpListener::bind("127.0.0.1:0").context("binding the websocket listener")?;
    let websocket_port = websocket_listener.local_addr()?.port();
    let bundle = Bundle {
        http: HttpDefault::connect().await.context("connecting http")?,
        keyvalue: KeyValueDefault::connect().await.context("connecting keyvalue")?,
        messaging: <MessagingDefault as Backend>::connect()
            .await
            .context("connecting messaging")?,
        websocket: WebSocketDefault::with_listener(websocket_listener)
            .context("connecting websocket")?,
    };

    let keyvalue = bundle.keyvalue.clone();
    let messaging = bundle.messaging.clone();

    let runtime = single_guest("conformance_wasm.wasm", bundle)
        .await?
        .host::<WasiHttp>()?
        .host::<WasiKeyValue>()?
        .host::<WasiMessaging>()?
        .host::<WasiWebSocket>()?
        .into_runtime()?;

    Ok(Fx {
        runtime,
        keyvalue,
        messaging,
        websocket_port,
    })
}

// --- outbound `wasi:http`: the guest's outbound request crosses the WIT
// --- boundary into the default backend's reqwest hook and reaches a real
// --- origin; the origin's response (and any failure) crosses back.

/// Drive the conformance guest's `/proxy` route and parse its JSON report.
async fn proxy(runtime: &Runtime<Bundle>, payload: &Value) -> Result<Value> {
    let response = http::post_json(runtime, "/proxy", payload.to_string()).await?;
    ensure!(response.status().is_success(), "the proxy route responds");
    serde_json::from_slice(response.body()).context("parsing the proxy report")
}

#[tokio::test(flavor = "multi_thread")]
async fn post_forwarded() -> Result<()> {
    let fx = fx().await?;
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(body_string("test body"))
        .respond_with(ResponseTemplate::new(201).set_body_string("Created"))
        .mount(&server)
        .await;

    let report = proxy(
        &fx.runtime,
        &json!({
            "url": server.uri(),
            "method": "POST",
            "body": "test body",
        }),
    )
    .await?;

    assert_eq!(report["status"], 201, "the origin's status crosses back: {report}");
    assert_eq!(report["body"], "Created", "the origin's body crosses back");

    let requests = server.received_requests().await.context("requests recorded")?;
    assert_eq!(requests.len(), 1);
    // The backend replaces any inherited Host header with the origin's:
    // exactly one, and it names the target.
    let hosts: Vec<_> = requests[0].headers.get_all("host").iter().collect();
    assert_eq!(hosts.len(), 1, "exactly one Host header reaches the origin");
    assert!(hosts[0].to_str()?.starts_with("127.0.0.1:"), "Host names the origin");

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn headers_forwarded() -> Result<()> {
    let fx = fx().await?;
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(header("X-Custom-Header", "custom-value"))
        .and(header("Authorization", "Bearer token123"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;

    let report = proxy(
        &fx.runtime,
        &json!({
            "url": server.uri(),
            "headers": [
                ["X-Custom-Header", "custom-value"],
                ["Authorization", "Bearer token123"],
            ],
        }),
    )
    .await?;

    // The mock only matches when both guest headers reached the origin.
    assert_eq!(report["status"], 200, "{report}");
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn forbidden_headers_stripped() -> Result<()> {
    let fx = fx().await?;
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("connection", "keep-alive")
                .insert_header("transfer-encoding", "chunked")
                .insert_header("upgrade", "websocket")
                .insert_header("content-type", "application/json")
                .insert_header("x-safe-header", "safe-value"),
        )
        .mount(&server)
        .await;

    let report = proxy(&fx.runtime, &json!({ "url": server.uri() })).await?;
    assert_eq!(report["status"], 200, "{report}");

    let names: Vec<&str> = report["headers"]
        .as_array()
        .context("headers reported")?
        .iter()
        .filter_map(|pair| pair[0].as_str())
        .collect();
    assert!(names.contains(&"content-type"), "permitted headers survive: {names:?}");
    assert!(names.contains(&"x-safe-header"), "permitted headers survive: {names:?}");
    for forbidden in ["connection", "transfer-encoding", "upgrade"] {
        assert!(!names.contains(&forbidden), "`{forbidden}` is stripped: {names:?}");
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn connection_refused() -> Result<()> {
    let fx = fx().await?;
    // Bind then drop a listener so the port is known-dead, instead of
    // assuming a fixed port is unused.
    let port = TcpListener::bind("127.0.0.1:0")?.local_addr()?.port();
    let report =
        proxy(&fx.runtime, &json!({ "url": format!("http://127.0.0.1:{port}/test") })).await?;
    assert!(report["error"].is_string(), "a refused connection surfaces: {report}");
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn bad_client_cert_rejected() -> Result<()> {
    let fx = fx().await?;
    let server = MockServer::start().await;
    Mock::given(method("GET")).respond_with(ResponseTemplate::new(200)).mount(&server).await;

    let report = proxy(
        &fx.runtime,
        &json!({
            "url": server.uri(),
            "headers": [["Client-Cert", "not-valid-base64!!!"]],
        }),
    )
    .await?;
    assert!(report["error"].is_string(), "undecodable base64 fails the request: {report}");

    // "invalid pem content" in base64: decodes, but is no certificate.
    let report = proxy(
        &fx.runtime,
        &json!({
            "url": server.uri(),
            "headers": [["Client-Cert", "aW52YWxpZCBwZW0gY29udGVudA=="]],
        }),
    )
    .await?;
    assert!(report["error"].is_string(), "a non-PEM certificate fails the request: {report}");

    assert!(
        server.received_requests().await.context("requests recorded")?.is_empty(),
        "no request leaves the host with a bad certificate"
    );
    Ok(())
}

// --- `wasi:keyvalue` CAS: the guest runs the full contract inside the route
// --- (set/get round-trip, a clean swap, then a stale swap whose `cas-failed`
// --- error carries a fresh handle the retry consumes) and fails the request
// --- on any deviation. The success response is those guest-side assertions
// --- crossing back; the fresh-handle threading is host machinery nothing
// --- else covers.

#[tokio::test(flavor = "multi_thread")]
async fn keyvalue_cas() -> Result<()> {
    let fx = fx().await?;
    let response = http::post(&fx.runtime, "/keyvalue?key=k1&cas=c1", "payload-value").await?;
    assert!(response.status().is_success(), "guest completes the CAS legs: {:?}", response.body());
    Ok(())
}

// --- `wasi:messaging`: the guest publishes to topic `a`, and a subscription
// --- taken on the host broker before the request receives that message —
// --- delivery is observable only host-side.

#[tokio::test(flavor = "multi_thread")]
async fn publish_reaches_subscriber() -> Result<()> {
    let fx = fx().await?;
    let payload = r#"{"hello":"broker"}"#;

    // Subscribe before publishing: the broadcast channel only delivers
    // messages sent after the receiver is taken.
    let client = fx.messaging.connect().await.context("connect broker client")?;
    let mut subscription = client.subscribe().await.context("subscribe")?;

    let response = http::post_json(&fx.runtime, "/messaging/pub-sub", payload).await?;
    assert!(response.status().is_success(), "guest publishes across the messaging boundary");

    let message = tokio::time::timeout(Duration::from_secs(5), subscription.next())
        .await
        .context("timed out waiting for the published message")?
        .context("subscription closed without a message")?;
    assert_eq!(message.topic, "a", "guest published to topic `a`");
    assert_eq!(message.payload, payload.as_bytes(), "the payload reached the broker intact");
    Ok(())
}

// --- `wasi:websocket`, both directions: the guest's `connect` + `send`
// --- crosses into the host and reaches a connected external peer, and a peer
// --- message travels back through the host into the guest's event handler.

#[tokio::test(flavor = "multi_thread")]
async fn peer_round_trip() -> Result<()> {
    let fx = fx().await?;

    // The websocket trigger loop forwards inbound peer messages to the guest
    // handler; spawn it the way a deployment's `run` would.
    let trigger = fx.runtime.clone();
    tokio::spawn(async move {
        if let Err(e) = omnia::Server::run(&WasiWebSocket, &trigger).await {
            eprintln!("websocket trigger loop failed: {e}");
        }
    });

    // The backend's server starts on a spawned task; retry until it accepts.
    let url = format!("ws://127.0.0.1:{}", fx.websocket_port);
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
        let response = http::post(&fx.runtime, "/websocket", "hello sockets").await?;
        assert!(response.status().is_success(), "guest connects and sends across the ws boundary");
        assert_eq!(
            serde_json::from_slice::<Value>(response.body())?,
            json!({ "message": "event sent" }),
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

    // Inbound leg: the peer's message must cross host -> guest handler,
    // which records it in the shared keyvalue bucket.
    peer.send(Message::Binary(b"ping from peer".as_slice().into())).await.context("peer send")?;
    let bucket = fx.keyvalue.open_bucket("omnia_bucket".to_owned()).await.context("open bucket")?;
    let mut recorded = None;
    for _ in 0..50 {
        if let Some(value) = bucket.get("ws-inbound".to_owned()).await.context("probe")? {
            recorded = Some(value);
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(
        recorded.as_deref(),
        Some(b"ping from peer".as_slice()),
        "the peer's message reached the guest handler"
    );

    Ok(())
}
