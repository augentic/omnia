//! End-to-end tests for `omnia:websocket`: every scenario runs a real guest
//! component from `crates/test-programs` through the omnia runtime. Command
//! exports are driven through `wasi:cli/run`; the `incoming-handler` export
//! is driven in-process through [`WebSocketHandler`], with the events the
//! default backend raises for real peers on a loopback port. The trigger's
//! server runs only where it alone decides the outcome: a stop mid-event, and
//! an event stream that ends. The guest asserts what it observes across the
//! boundary (and traps on failure); the host side asserts what each peer
//! receives, and what reached a faulting backend.

#![cfg(not(target_arch = "wasm32"))]

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use futures::future::BoxFuture;
use futures::{FutureExt as _, SinkExt as _, StreamExt as _};
use omnia::{
    AcquireError, ExitStatus, FutureResult, Grants, GuestEntry, GuestId, Host, Provides,
    RegistrySource, Runtime, RuntimeOptions,
};
use omnia_test::host::{Deployment, Metrics};
use omnia_wasi_otel::{OtelClient, OtelDefault, WasiOtel};
use omnia_wasi_websocket::{
    Endpoint, Error, Event, Events, WasiWebSocket, WebSocketClient, WebSocketDefault,
    WebSocketHandler,
};
use tokio::net::TcpStream;
use tokio::sync::watch;
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
    let metrics = Metrics::install();
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
    assert_eq!(metrics.count("processing_errors"), 1);

    // the guest's own refusal comes back as the handler's error
    let mut first = connect(addr).await;
    first.send(Message::text("")).await.expect("first sends");
    let refused = handler.handle(next(&mut events).await).await.expect_err("refused");
    assert!(format!("{refused:#}").contains("an empty message"), "{refused:#}");
    assert_eq!(metrics.count("processing_errors"), 2);

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

    // every event counts, whether or not it was handled
    assert_eq!(metrics.count("event_counter"), 4);
    runtime.shutdown();
}

#[tokio::test]
async fn full_gate() {
    let metrics = Metrics::install();
    let (server, addr, mut events) = serve().await;
    let runtime = Deployment::new()
        .guest("guest", test_programs::WEBSOCKET_HANDLER_ECHO)
        .options(RuntimeOptions {
            max_concurrent_invocations: Some(1),
            ..RuntimeOptions::default()
        })
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

    let mut peer = connect(addr).await;
    peer.send(Message::text("ping")).await.expect("the peer sends");
    let ping = next(&mut events).await;

    // with the only slot held, the event fails before a store is built
    let guest = runtime.guest(&GuestId::from("guest")).await.expect("the guest loads");
    let lease = runtime.try_lease(guest).expect("the gate is empty");
    let built = runtime.stores_built();
    let refused = handler.handle(ping.clone()).await.expect_err("the gate is full");
    assert!(format!("{refused:#}").contains("MAX_CONCURRENT_INVOCATIONS"), "{refused:#}");
    assert_eq!(runtime.stores_built(), built);

    // freed, the same event is handled
    drop(lease);
    handler.handle(ping).await.expect("handled");
    assert_eq!(receive(&mut peer).await, Message::text("ping"));

    // the refusal counts as refused, never as a failure, and both deliveries count
    assert_eq!(metrics.count("invocations_refused"), 1);
    assert_eq!(metrics.count("processing_errors"), 0);
    assert_eq!(metrics.count("event_counter"), 2);

    quiet(&mut peer).await;
    runtime.shutdown();
}

// The stop lands while the guest pauses over `slow`. Its echo still reaches
// the peer, the message sent after the stop is never read, and `run` returns
// once the event is handled.
#[tokio::test]
async fn stop_in_flight() {
    let (server, addr, _) = serve().await;
    let runtime = Deployment::new()
        .guest("guest", test_programs::WEBSOCKET_HANDLER_ECHO)
        .boot(Backends::new(server), |deployment| {
            deployment.host::<WasiWebSocket>()?;
            deployment.host::<WasiOtel>()?;
            Ok(())
        })
        .await
        .expect("runtime boots");
    let built = runtime.stores_built();
    let mut peer = connect(addr).await;

    let stop = async {
        peer.send(Message::text("slow")).await.expect("the peer sends");
        in_flight(&runtime).await;
        runtime.stop();
        peer.send(Message::text("late")).await.expect("the peer sends");
    };
    let (outcome, ()) = tokio::join!(WasiWebSocket::run(&runtime), stop);
    outcome.expect("a stopped server returns Ok");

    assert_eq!(receive(&mut peer).await, Message::text("slow"));
    quiet(&mut peer).await;
    assert_eq!(runtime.stores_built(), built + 1);

    runtime.shutdown();
}

// resolves once an invocation holds a slot of the concurrency gate
async fn in_flight<B: omnia::Backends>(runtime: &Runtime<B>) {
    let held = async {
        while runtime.invocations().in_flight == 0 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    };
    tokio::time::timeout(Duration::from_secs(5), held).await.expect("an invocation takes a slot");
}

// wraps the default backend and ends its event stream once `ended` is set, as
// a backend that loses its transport mid-run does
#[derive(Clone, Debug)]
struct Ended {
    inner: WebSocketDefault,
    ended: watch::Receiver<bool>,
}

impl WebSocketClient for Ended {
    fn endpoint(&self) -> FutureResult<Arc<dyn Endpoint>> {
        let endpoint = self.clone();
        async move { Ok(Arc::new(endpoint) as Arc<dyn Endpoint>) }.boxed()
    }
}

impl Endpoint for Ended {
    fn events(&self) -> FutureResult<Events> {
        let events = self.inner.events();
        let mut ended = self.ended.clone();
        async move {
            let cut = async move {
                let _ = ended.wait_for(|&done| done).await;
            };
            Ok(Box::pin(events.await?.take_until(cut)) as Events)
        }
        .boxed()
    }

    fn send(&self, event: Event, sockets: Option<Vec<String>>) -> FutureResult<()> {
        self.inner.send(event, sockets)
    }

    fn broadcast(&self, event: Event, except: Vec<String>) -> FutureResult<()> {
        self.inner.broadcast(event, except)
    }

    fn exists(&self, connection: String) -> FutureResult<bool> {
        self.inner.exists(connection)
    }
}

// The event stream ends while the guest pauses over `slow`. Its echo still
// reaches the peer, the message sent after is never read, and `run` then
// fails so the orchestrator restarts the process.
#[tokio::test]
async fn events_end() {
    let (server, addr, _) = serve().await;
    let (end, ended) = watch::channel(false);
    let runtime = Deployment::new()
        .guest("guest", test_programs::WEBSOCKET_HANDLER_ECHO)
        .boot(Backends::new(Ended { inner: server, ended }), |deployment| {
            deployment.host::<WasiWebSocket>()?;
            deployment.host::<WasiOtel>()?;
            Ok(())
        })
        .await
        .expect("runtime boots");
    let built = runtime.stores_built();
    let mut peer = connect(addr).await;

    let cut = async {
        peer.send(Message::text("slow")).await.expect("the peer sends");
        in_flight(&runtime).await;
        end.send_replace(true);
        peer.send(Message::text("late")).await.expect("the peer sends");
    };
    let (outcome, ()) = tokio::join!(WasiWebSocket::run(&runtime), cut);
    let error = outcome.expect_err("an ended event stream fails the server");
    assert!(format!("{error:#}").contains("event stream ended"), "unexpected error: {error:#}");

    assert_eq!(receive(&mut peer).await, Message::text("slow"));
    quiet(&mut peer).await;
    assert_eq!(runtime.stores_built(), built + 1);

    runtime.shutdown();
}

const DEADLINE_ECHO_PACKAGE: &str = "test:deadline-echo@1.0.0";

// a registry holding one release, the deadline echo guest's, which it
// answers `delay` after it is asked
struct SlowRegistry {
    bytes: Vec<u8>,
    delay: Duration,
}

impl RegistrySource for SlowRegistry {
    fn acquire<'a>(
        &'a self, package: &'a str, endpoint: Option<&'a str>, grants: &'a Grants,
    ) -> BoxFuture<'a, Result<Vec<u8>, AcquireError>> {
        Box::pin(async move {
            assert_eq!(package, DEADLINE_ECHO_PACKAGE);
            assert_eq!(endpoint, None, "a declared package names no registry of its own");
            assert_eq!(
                *grants,
                Grants::all(),
                "a declared package is the deployment's word, held to no guest's grants"
            );
            tokio::time::sleep(self.delay).await;
            Ok(self.bytes.clone())
        })
    }
}

// A declared guest loads at its first event, and the registry takes `LOAD` to
// answer, so a deadline counted from the slot lies at least `LOAD` past the
// event's arrival.
#[tokio::test]
async fn websocket_deadline_echo() {
    const LOAD: Duration = Duration::from_millis(300);
    const GUEST_TIMEOUT: Duration = Duration::from_secs(30);

    let (server, addr, mut events) = serve().await;
    let bytes = std::fs::read(test_programs::WEBSOCKET_DEADLINE_ECHO).expect("guest bytes");
    let runtime = Deployment::new()
        .entry(GuestEntry::package(DEADLINE_ECHO_PACKAGE).route_websocket("deadline"))
        .guest_timeout(GUEST_TIMEOUT)
        .boot(Backends::new(server), |deployment| {
            deployment.registry_source(SlowRegistry { bytes, delay: LOAD });
            deployment.host::<WasiWebSocket>()?;
            deployment.host::<WasiOtel>()?;
            Ok(())
        })
        .await
        .expect("runtime boots");
    let handler = WebSocketHandler::new(&runtime)
        .expect("websocket routes consistent")
        .expect("a guest takes the websocket route");

    // the default backend sets no route, so the test names the guest's
    let mut peer = connect(addr).await;
    peer.send(Message::text("deadline")).await.expect("the peer sends");
    let mut event = next(&mut events).await;
    event.route = Some("deadline".to_owned());

    let earliest = epoch_millis(SystemTime::now() + LOAD + GUEST_TIMEOUT);
    handler.handle(event).await.expect("handled");
    let latest = epoch_millis(SystemTime::now() + GUEST_TIMEOUT);

    let Message::Text(deadline) = receive(&mut peer).await else {
        panic!("the deadline reaches the peer as text");
    };
    let deadline: u64 = deadline.as_str().parse().expect("milliseconds since the epoch");
    assert!((earliest..=latest).contains(&deadline), "{deadline}");

    quiet(&mut peer).await;
    runtime.shutdown();
}

fn epoch_millis(instant: SystemTime) -> u64 {
    let since_epoch = instant.duration_since(UNIX_EPOCH).expect("after the epoch");
    u64::try_from(since_epoch.as_millis()).expect("fits in u64 milliseconds")
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
