//! End-to-end tests for `wasi:messaging`: every scenario runs a real guest
//! component from `crates/test-programs` through the omnia runtime over the
//! in-memory broker. Command exports are driven through `wasi:cli/run`; the
//! `incoming-handler` export is driven in-process through
//! [`MessagingHandler`], bypassing the server loop. The guest asserts what it
//! observes across the boundary (and traps on failure); the host side asserts
//! what reached the broker, from a subscription opened before the guest runs,
//! and what the host told the backend through the message's [`Ack`] token.

#![cfg(not(target_arch = "wasm32"))]

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use futures::{FutureExt as _, StreamExt as _};
use omnia::{ExitStatus, GuestEntry, Server as _};
use omnia_test::host::{Backends, Deployment};
use omnia_wasi_messaging::{
    Ack, Client, FutureResult, Message, MessagingHandler, Reply, RequestOptions, Subscriptions,
    WasiMessaging, WasiMessagingCtx,
};
use omnia_wasi_otel::WasiOtel;

// Every guest program in `crates/test-programs` must have a matching test
// here; a new program without one fails to compile.
test_programs::foreach_messaging!();

// The backend's side of the ack seam: records whether the host called it.
#[derive(Debug, Default)]
struct RecordingAck(AtomicBool);

impl Ack for RecordingAck {
    fn ack(&self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

impl RecordingAck {
    fn attach(message: &mut Message) -> Arc<Self> {
        let ack = Arc::new(Self::default());
        message.ack = Some(Arc::clone(&ack) as Arc<dyn Ack>);
        ack
    }

    fn acked(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

// A backend whose subscription has already ended, as a consumer librdkafka
// has given up on would present it; everything else is a no-op.
#[derive(Clone, Debug)]
struct EndedBroker;

impl WasiMessagingCtx for EndedBroker {
    fn connect(&self) -> FutureResult<Arc<dyn Client>> {
        async { Ok(Arc::new(Self) as Arc<dyn Client>) }.boxed()
    }
}

impl Client for EndedBroker {
    fn subscribe(&self) -> FutureResult<Subscriptions> {
        async { Ok(Box::pin(futures::stream::empty()) as Subscriptions) }.boxed()
    }

    fn send(&self, _topic: String, _message: Message) -> FutureResult<()> {
        async { Ok(()) }.boxed()
    }

    fn request(
        &self, _topic: String, _message: Message, _options: Option<RequestOptions>,
    ) -> FutureResult<Message> {
        async { Ok(Message::default()) }.boxed()
    }
}

async fn run_guest(wasm: &str, backends: Backends) {
    let status = Deployment::new()
        .guest("guest", wasm)
        .run_host::<WasiMessaging, _>(backends)
        .await
        .expect("guest runs");
    assert_eq!(status, ExitStatus::SUCCESS, "guest `{wasm}` failed");
}

async fn next(stream: &mut Subscriptions) -> Message {
    tokio::time::timeout(Duration::from_secs(5), stream.next())
        .await
        .expect("a message reaches the subscription")
        .expect("the broker stays open")
}

fn metadata(message: &Message) -> Vec<(String, String)> {
    let mut pairs: Vec<_> = message
        .metadata
        .as_ref()
        .map(|md| md.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
        .unwrap_or_default();
    pairs.sort();
    pairs
}

#[tokio::test]
async fn messaging_produce_handle() {
    let backends = Backends::defaults().await;
    // The in-memory broker fans out only to live subscribers: subscribe
    // before the guest publishes, or the message is dropped.
    let mut stream = backends.messaging.subscribe().await.expect("subscribe");

    // The guest exports both `wasi:cli/run` and the incoming handler: boot
    // without driving either, then drive each export in turn.
    let runtime = Deployment::new()
        .guest("guest", test_programs::MESSAGING_PRODUCE_HANDLE)
        .boot(backends, |deployment| {
            deployment.host::<WasiMessaging, Backends>()?;
            deployment.host::<WasiOtel, Backends>()?;
            Ok(())
        })
        .await
        .expect("runtime boots");
    let handler = MessagingHandler::new(&runtime)
        .expect("messaging routes consistent")
        .expect("the guest exports the messaging handler");

    // Producer: the one message the command sent reached the broker with its
    // topic, payload, and metadata intact.
    let status = runtime.run_command().await.expect("command runs");
    assert_eq!(status, ExitStatus::SUCCESS);
    let mut sent = next(&mut stream).await;
    assert_eq!(sent.topic, "orders");
    assert_eq!(sent.payload, b"order-1");
    assert_eq!(metadata(&sent), [("key".to_owned(), "value".to_owned())]);
    assert!(sent.reply.is_none(), "a plain send names no reply topic");

    // Incoming handler: the broker's message, now carrying a reply topic,
    // dispatches through the sole-exporter catch-all to the guest, which
    // asserts its fields and replies on that topic. The guest returned `Ok`,
    // so the host acks the backend's token.
    sent.reply = Some(Reply {
        topic: "orders.reply".to_owned(),
    });
    let ack = RecordingAck::attach(&mut sent);
    handler.handle(sent).await.expect("handled");
    let reply = next(&mut stream).await;
    assert_eq!(reply.topic, "orders.reply");
    assert_eq!(reply.payload, b"handled");
    assert!(ack.acked(), "a guest that returned `Ok` ran");

    runtime.shutdown();
}

// A guest's returned `Err` is the handler's `Err`, carrying the guest's
// message, rather than being flattened into success; and the guest ran, so
// the token is acked: the failure is logged and counted, never redelivered.
#[tokio::test]
async fn messaging_handle_err() {
    let runtime = Deployment::new()
        .guest("guest", test_programs::MESSAGING_HANDLE_ERR)
        .boot(Backends::defaults().await, |deployment| {
            deployment.host::<WasiMessaging, Backends>()?;
            deployment.host::<WasiOtel, Backends>()?;
            Ok(())
        })
        .await
        .expect("runtime boots");
    let handler = MessagingHandler::new(&runtime)
        .expect("messaging routes consistent")
        .expect("the guest exports the messaging handler");

    let mut message = Message::new(b"order-1".to_vec());
    message.topic = "orders".to_owned();
    let ack = RecordingAck::attach(&mut message);
    let err = handler.handle(message).await.expect_err("the guest's error is the handler's");
    assert!(format!("{err:#}").contains("rejected orders"), "unexpected error: {err:#}");
    assert!(ack.acked(), "a guest that returned `Err` still ran");

    runtime.shutdown();
}

// A guest that traps ran: `handle` fails with the trap, and the token is
// acked, since the host neither retries nor holds the record back.
#[tokio::test]
async fn guest_trap_acked() {
    let runtime = Deployment::new()
        .guest("guest", test_programs::MESSAGING_PRODUCE_HANDLE)
        .boot(Backends::defaults().await, |deployment| {
            deployment.host::<WasiMessaging, Backends>()?;
            deployment.host::<WasiOtel, Backends>()?;
            Ok(())
        })
        .await
        .expect("runtime boots");
    let handler = MessagingHandler::new(&runtime)
        .expect("messaging routes consistent")
        .expect("the guest exports the messaging handler");

    // the guest asserts on `order-1`; any other payload trips its assert
    let mut message = Message::new(b"not-the-order".to_vec());
    message.topic = "orders".to_owned();
    let ack = RecordingAck::attach(&mut message);
    let err = handler.handle(message).await.expect_err("a trapping guest is an error");
    assert!(
        !format!("{err:#}").contains("guest returned an error"),
        "a trap is not a returned `Err`: {err:#}"
    );
    assert!(ack.acked(), "a guest that trapped still ran");

    runtime.shutdown();
}

// Explicit routes: `guest` handles `orders`, and `absent` — declared from a
// path that does not exist, so it loads (and fails) at first use — handles
// `absent`; any other topic has no route.
async fn routed_runtime() -> omnia::Runtime<Backends> {
    let absent = std::env::temp_dir().join("omnia-messaging-absent-guest.wasm");
    Deployment::new()
        .entry(
            GuestEntry::new("guest", test_programs::MESSAGING_PRODUCE_HANDLE)
                .route_messaging("orders"),
        )
        .entry(GuestEntry::new("absent", absent).route_messaging("absent"))
        .boot(Backends::defaults().await, |deployment| {
            deployment.host::<WasiMessaging, Backends>()?;
            deployment.host::<WasiOtel, Backends>()?;
            Ok(())
        })
        .await
        .expect("runtime boots")
}

// A topic no route matches has nothing to run: the message is dropped with
// `Ok`, and the token is acked so the backend does not hold the record back.
#[tokio::test]
async fn no_route_acked() {
    let runtime = routed_runtime().await;
    let handler = MessagingHandler::new(&runtime)
        .expect("messaging routes consistent")
        .expect("a routed guest exports the messaging handler");

    let mut message = Message::new(b"nobody home".to_vec());
    message.topic = "nowhere".to_owned();
    let ack = RecordingAck::attach(&mut message);
    handler.handle(message).await.expect("an unrouted topic is dropped, not an error");
    assert!(ack.acked(), "nothing to run is acked");

    runtime.shutdown();
}

// A routed guest that cannot be loaded never runs: `handle` fails, and the
// token is dropped uncalled so the backend knows no guest saw the record.
#[tokio::test]
async fn unloadable_guest_unacked() {
    let runtime = routed_runtime().await;
    let handler = MessagingHandler::new(&runtime)
        .expect("messaging routes consistent")
        .expect("a routed guest exports the messaging handler");

    let mut message = Message::new(b"never seen".to_vec());
    message.topic = "absent".to_owned();
    let ack = RecordingAck::attach(&mut message);
    let err = handler.handle(message).await.expect_err("an unloadable guest is an error");
    assert!(format!("{err:#}").contains("absent"), "unexpected error: {err:#}");
    assert!(!ack.acked(), "no guest ran, so nothing is acked");

    runtime.shutdown();
}

// A subscription that ends is a server failure, not a clean exit: a backend
// that gives up on its consumer must take the process down for a restart
// rather than leave it idle and looking healthy.
#[tokio::test]
async fn subscription_end_fails_server() {
    let backends = Backends::defaults().await.messaging(EndedBroker);
    let runtime = Deployment::new()
        .guest("guest", test_programs::MESSAGING_PRODUCE_HANDLE)
        .boot(backends, |deployment| {
            deployment.host::<WasiMessaging, Backends>()?;
            deployment.host::<WasiOtel, Backends>()?;
            Ok(())
        })
        .await
        .expect("runtime boots");

    let err =
        WasiMessaging.run(&runtime).await.expect_err("an ended subscription fails the server");
    assert!(format!("{err:#}").contains("subscription ended"), "unexpected error: {err:#}");

    runtime.shutdown();
}

#[tokio::test]
async fn messaging_request_reply() {
    let backends = Backends::defaults().await;
    let mut stream = backends.messaging.subscribe().await.expect("subscribe");

    // `backends` outlives the run so the broker stays open for the check
    // below; a closed broker ends the stream instead of staying quiet.
    run_guest(test_programs::MESSAGING_REQUEST_REPLY, backends.clone()).await;

    // Wire fidelity: the request was published on its topic with the content
    // type folded into metadata; the reply to a message without a reply
    // topic never reached the broker.
    let request = next(&mut stream).await;
    assert_eq!(request.topic, "ping");
    assert_eq!(request.payload, b"ping");
    assert_eq!(metadata(&request), [("content-type".to_owned(), "text/plain".to_owned())]);
    assert!(
        tokio::time::timeout(Duration::from_millis(200), stream.next()).await.is_err(),
        "nothing else reached the broker"
    );
}
