//! End-to-end tests for `omnia:websocket`: every scenario runs a real guest
//! component from `crates/test-programs` through the omnia runtime. Command
//! exports are driven through `wasi:cli/run`; the `incoming-handler` export
//! is driven in-process through [`WebSocketHandler`], with the events the
//! default backend raises for real peers on a loopback port. The guest
//! asserts what it observes across the boundary (and traps on failure); the
//! host side asserts what each peer receives, and what reached a faulting
//! backend.

#![cfg(not(target_arch = "wasm32"))]

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::{FutureExt as _, SinkExt as _, StreamExt as _};
use omnia::{ExitStatus, FutureResult, Provides};
use omnia_test::host::Deployment;
use omnia_wasi_otel::{OtelClient, OtelDefault, WasiOtel};
use omnia_wasi_websocket::{
    Endpoint, Error, Event, Events, WasiWebSocket, WebSocketClient, WebSocketDefault,
    WebSocketHandler,
};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::protocol::CloseFrame;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async};

// Every guest program in `crates/test-programs` must have a matching test
// here; a new program without one fails to compile.
test_programs::foreach_websocket!();

// `omnia_test`'s bundle holds the default backend alone, and the error row
// runs over a faulting one
#[derive(Clone, Debug)]
struct Backends<W> {
    websocket: W,
    otel: OtelDefault,
}

impl<W> Backends<W> {
    const fn new(websocket: W) -> Self {
        Self {
            websocket,
            otel: OtelDefault,
        }
    }
}

impl<W: WebSocketClient + Clone> Provides<WasiWebSocket> for Backends<W> {
    fn borrow(&mut self) -> &dyn WebSocketClient {
        &self.websocket
    }
}

impl<W: WebSocketClient + Clone> Provides<WasiOtel> for Backends<W> {
    fn borrow(&mut self) -> &dyn OtelClient {
        &self.otel
    }
}

type Peer = WebSocketStream<MaybeTlsStream<TcpStream>>;

// the default backend serving a loopback port, and its event stream, opened
// before any peer speaks
async fn serve() -> (WebSocketDefault, SocketAddr, Events) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind a loopback port");
    let addr = listener.local_addr().expect("the bound address");
    let server = WebSocketDefault::with_listener(listener).expect("serve the listener");
    let endpoint = server.endpoint().await.expect("endpoint");
    let events = endpoint.events().await.expect("events");
    (server, addr, events)
}

async fn connect(addr: SocketAddr) -> Peer {
    let (peer, _response) = connect_async(format!("ws://{addr}")).await.expect("peer connects");
    peer
}

async fn next(events: &mut Events) -> Event {
    tokio::time::timeout(Duration::from_secs(5), events.next())
        .await
        .expect("an event reaches the backend")
        .expect("the event stream stays open")
}

async fn receive(peer: &mut Peer) -> Message {
    tokio::time::timeout(Duration::from_secs(5), peer.next())
        .await
        .expect("a frame reaches the peer")
        .expect("the connection stays open")
        .expect("a well-formed frame")
}

async fn quiet(peer: &mut Peer) {
    assert!(
        tokio::time::timeout(Duration::from_millis(200), peer.next()).await.is_err(),
        "nothing else reached the peer"
    );
}

#[tokio::test]
async fn websocket_handler_echo() {
    let (server, addr, mut events) = serve().await;
    // The handler guest exports no `wasi:cli/run`: boot without driving it
    // and dispatch the backend's events through the trigger's in-process
    // handler.
    let runtime = Deployment::new()
        .guest("guest", test_programs::WEBSOCKET_HANDLER_ECHO)
        .boot(Backends::new(server), |deployment| {
            deployment.host::<WasiWebSocket>()?;
            deployment.host::<WasiOtel>()?;
            Ok(())
        })
        .await
        .expect("runtime boots");
    let handler = WebSocketHandler::new(&runtime)
        .expect("websocket routes consistent")
        .expect("the guest exports the websocket handler");

    // an event that names no connection never reaches the guest
    let anonymous = handler.handle(Event::new(b"ping".to_vec())).await;
    assert!(anonymous.is_err(), "an anonymous event is refused");

    // the guest's own refusal comes back as the handler's error
    let mut first = connect(addr).await;
    first.send(Message::text("")).await.expect("first sends");
    let refused = handler.handle(next(&mut events).await).await.expect_err("refused");
    assert!(format!("{refused:#}").contains("an empty message"), "{refused:#}");

    // text comes back to its sender as text
    first.send(Message::text("ping")).await.expect("first sends");
    let ping = next(&mut events).await;
    handler.handle(ping.clone()).await.expect("handled");
    assert_eq!(receive(&mut first).await, Message::text("ping"));

    // bytes reach their sender, and every other open connection, as bytes
    let mut second = connect(addr).await;
    second.send(Message::binary(vec![1, 2])).await.expect("second sends");
    let bytes = next(&mut events).await;
    assert_ne!(bytes.socket_addr, ping.socket_addr, "each connection has its own id");
    handler.handle(bytes).await.expect("handled");
    assert_eq!(receive(&mut second).await, Message::binary(vec![1, 2]));
    assert_eq!(receive(&mut first).await, Message::binary(vec![1, 2]));

    quiet(&mut first).await;
    quiet(&mut second).await;
    runtime.shutdown();
}

#[tokio::test]
async fn websocket_close() {
    let (server, addr, mut events) = serve().await;

    // each peer speaks first, so the test learns its connection's id
    let mut first = connect(addr).await;
    first.send(Message::text("hello")).await.expect("the first peer sends");
    let one = next(&mut events).await.socket_addr.expect("the event names its connection");
    let mut second = connect(addr).await;
    second.send(Message::text("hello")).await.expect("the second peer sends");
    let two = next(&mut events).await.socket_addr.expect("the event names its connection");

    let status = Deployment::new()
        .guest("guest", test_programs::WEBSOCKET_CLOSE)
        .args([one, two])
        .run_host::<WasiWebSocket, _>(Backends::new(server))
        .await
        .expect("guest runs");
    assert_eq!(status, ExitStatus::SUCCESS, "guest failed");

    let done = CloseFrame {
        code: CloseCode::Normal,
        reason: "done".into(),
    };
    assert_eq!(receive(&mut first).await, Message::text("both"));
    assert_eq!(receive(&mut first).await, Message::Close(Some(done)));
    assert_eq!(receive(&mut second).await, Message::text("both"));
    quiet(&mut second).await;
}

// fails every send with the kind its first connection is named for,
// recording each send's connections so the run's exchanges are pinned
#[derive(Clone, Debug, Default)]
struct Faulting {
    sent: Arc<Mutex<Vec<Vec<String>>>>,
}

impl WebSocketClient for Faulting {
    fn endpoint(&self) -> FutureResult<Arc<dyn Endpoint>> {
        let endpoint = self.clone();
        async move { Ok(Arc::new(endpoint) as Arc<dyn Endpoint>) }.boxed()
    }
}

impl Endpoint for Faulting {
    fn events(&self) -> FutureResult<Events> {
        unreachable!("a command-only guest leaves the trigger inert")
    }

    fn send(&self, _event: Event, sockets: Option<Vec<String>>) -> FutureResult<()> {
        let sockets = sockets.expect("a send names its connections");
        self.sent.lock().expect("sent connections").push(sockets.clone());
        let fault = match sockets.first().map(String::as_str) {
            Some("not-found") => Error::NotFound("not found by the suite".to_owned()).into(),
            Some("denied") => Error::Denied("denied by the suite".to_owned()).into(),
            Some("invalid") => Error::Invalid("invalid in the suite".to_owned()).into(),
            Some("unsupported") => Error::Unsupported("unsupported by the suite".to_owned()).into(),
            Some("unavailable") => Error::Unavailable("unavailable in the suite".to_owned()).into(),
            _ => anyhow::anyhow!("an untyped backend failure"),
        };
        async move { Err(fault) }.boxed()
    }
}

#[tokio::test]
async fn websocket_error_codes() {
    let backends = Backends::new(Faulting::default());

    let status = Deployment::new()
        .guest("guest", test_programs::WEBSOCKET_ERROR_CODES)
        .run_host::<WasiWebSocket, _>(backends.clone())
        .await
        .expect("guest runs");
    assert_eq!(status, ExitStatus::SUCCESS, "guest failed");

    // the host refused every other call before it reached the backend, and
    // a send to two connections reached it once
    let sent = backends.websocket.sent.lock().expect("sent connections").clone();
    assert_eq!(
        sent,
        [
            vec!["not-found"],
            vec!["denied"],
            vec!["invalid"],
            vec!["unsupported"],
            vec!["unavailable"],
            vec!["other"],
            vec!["unavailable", "other"],
        ]
    );
}
