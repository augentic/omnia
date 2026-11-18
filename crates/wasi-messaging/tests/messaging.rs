//! End-to-end tests for `omnia:messaging`: every scenario runs a real guest
//! component from `crates/test-programs` through the omnia runtime over the
//! in-memory broker. Command exports are driven through `wasi:cli/run`; the
//! `incoming-handler` export is driven in-process through
//! [`MessagingHandler`], bypassing the backend's delivery loop, and once
//! through the default broker's own [`Endpoint::consume`]. The guest asserts
//! what it observes across the boundary (and traps on failure); the host
//! side asserts what reached the broker, from a subscription opened before
//! the guest runs, and the typed outcome the handler returns.

#![cfg(not(target_arch = "wasm32"))]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::{FutureExt as _, StreamExt as _};
use omnia::{ExitStatus, GuestEntry, Host, Runtime};
use omnia_test::host::{Backends, Deployment};
use omnia_wasi_messaging::{
    Endpoint, Error, FutureResult, HandleError, Handler, Message, MessagingClient,
    MessagingHandler, Metadata, Reply, RequestOptions, WasiMessaging,
};
use omnia_wasi_otel::WasiOtel;

// Every guest program in `crates/test-programs` must have a matching test
// here; a new program without one fails to compile.
test_programs::foreach_messaging!();

// A backend whose consumer has already given up, as librdkafka presents a
// consumer it has abandoned; everything else is a no-op.
#[derive(Clone, Debug)]
struct EndedBroker;

impl MessagingClient for EndedBroker {
    fn endpoint(&self) -> FutureResult<Arc<dyn Endpoint>> {
        async { Ok(Arc::new(Self) as Arc<dyn Endpoint>) }.boxed()
    }
}

impl Endpoint for EndedBroker {
    fn consume(&self, _handler: Arc<dyn Handler>) -> FutureResult<()> {
        async { Ok(()) }.boxed()
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

async fn boot<B>(deployment: &Deployment, backends: B) -> Runtime<B>
where
    B: omnia::Backends,
    WasiMessaging: Host<B>,
    WasiOtel: Host<B>,
{
    deployment
        .boot(backends, |deployment| {
            deployment.host::<WasiMessaging>()?;
            deployment.host::<WasiOtel>()?;
            Ok(())
        })
        .await
        .expect("runtime boots")
}

fn handler(runtime: &Runtime<Backends>) -> MessagingHandler<Backends> {
    MessagingHandler::new(runtime)
        .expect("messaging routes consistent")
        .expect("a guest exports the messaging handler")
}

// `wasm` as the sole guest, so it is the catch-all for every topic.
fn sole(wasm: &str) -> Deployment {
    Deployment::new().guest("guest", wasm)
}

// Explicit routes: `guest` handles `orders`, and `absent` — declared from a
// path that does not exist, so it loads (and fails) at first use — handles
// `absent`; any other topic has no route.
fn routed() -> Deployment {
    let absent = std::env::temp_dir().join("omnia-messaging-absent-guest.wasm");
    Deployment::new()
        .entry(
            GuestEntry::new("guest", test_programs::MESSAGING_PRODUCE_HANDLE)
                .route_messaging("orders"),
        )
        .entry(GuestEntry::new("absent", absent).route_messaging("absent"))
}

fn order(topic: &str) -> Message {
    let mut message = Message::new(b"order-1".to_vec());
    topic.clone_into(&mut message.topic);
    message
}

async fn run_guest(wasm: &str, backends: Backends) {
    let status = Deployment::new()
        .guest("guest", wasm)
        .run_host::<WasiMessaging, _>(backends)
        .await
        .expect("guest runs");
    assert_eq!(status, ExitStatus::SUCCESS, "guest `{wasm}` failed");
}

async fn next(stream: &mut omnia_wasi_messaging::Subscriptions) -> Message {
    tokio::time::timeout(Duration::from_secs(5), stream.next())
        .await
        .expect("a message reaches the subscription")
        .expect("the broker stays open")
}

fn metadata(message: &Message) -> Vec<(String, String)> {
    let mut pairs: Vec<_> = message
        .metadata
        .as_ref()
        .map_or_default(|md| md.iter().map(|(k, v)| (k.clone(), v.clone())).collect());
    pairs.sort();
    pairs
}

#[tokio::test]
async fn messaging_produce_handle() {
    let backends = Backends::defaults().await;
    // The in-memory broker fans out only to live subscribers: subscribe
    // before the guest publishes, or the message is dropped.
    let mut stream = backends.messaging.subscribe();

    // The guest exports both `wasi:cli/run` and the incoming handler: boot
    // without driving either, then drive each export in turn.
    let runtime = boot(&sole(test_programs::MESSAGING_PRODUCE_HANDLE), backends).await;
    let handler = handler(&runtime);

    // Producer: the one message the command sent reached the broker on its
    // topic, its content type and key carried as the metadata entries of
    // those names, beside the guest's own entry.
    let status = runtime.run_command().await.expect("command runs");
    assert_eq!(status, ExitStatus::SUCCESS);
    let mut sent = next(&mut stream).await;
    assert_eq!(sent.topic, "orders");
    assert_eq!(sent.payload, b"order-1");
    assert_eq!(
        metadata(&sent),
        [
            ("content-type".to_owned(), "text/plain".to_owned()),
            ("key".to_owned(), "customer-1".to_owned()),
            ("tenant".to_owned(), "acme".to_owned()),
        ]
    );
    assert!(sent.reply.is_none(), "a plain send names no reply topic");

    // Incoming handler: the broker's message, now carrying a reply topic,
    // dispatches through the sole-exporter catch-all to the guest, which
    // asserts its typed fields came back and replies to that address.
    sent.reply = Some(Reply {
        topic: "orders.reply".to_owned(),
    });
    handler.handle(sent).await.expect("handled");
    let reply = next(&mut stream).await;
    assert_eq!(reply.topic, "orders.reply");
    assert_eq!(reply.payload, b"handled");

    runtime.shutdown();
}

#[tokio::test]
async fn messaging_keyed() {
    let backends = Backends::defaults().await;
    let mut stream = backends.messaging.subscribe();
    let runtime = boot(&sole(test_programs::MESSAGING_KEYED), backends).await;
    let handler = handler(&runtime);

    // Producer: the message's key crossed as the `key` metadata entry,
    // beside the header the guest set.
    let status = runtime.run_command().await.expect("command runs");
    assert_eq!(status, ExitStatus::SUCCESS);
    let mut sent = next(&mut stream).await;
    assert_eq!(sent.topic, "orders");
    assert_eq!(sent.payload, b"order-7");
    assert_eq!(
        metadata(&sent),
        [("key".to_owned(), "customer-7".to_owned()), ("tenant".to_owned(), "acme".to_owned())]
    );

    // Incoming handler: the guest's router finds the key on the delivery, and
    // the host's deadline replaces one the delivery carried, whatever its case.
    sent.metadata.get_or_insert_default().insert("Deadline".to_owned(), "1".to_owned());
    handler.handle(sent).await.expect("handled");

    runtime.shutdown();
}

// A guest's returned `Err` comes back as its own outcome, carrying the
// guest's error, rather than flattened into success or mistaken for a trap.
#[tokio::test]
async fn messaging_handle_err() {
    let runtime =
        boot(&sole(test_programs::MESSAGING_HANDLE_ERR), Backends::defaults().await).await;
    let handler = handler(&runtime);

    let error =
        handler.handle(order("orders")).await.expect_err("the guest's error is the handler's");
    assert!(
        matches!(&error, HandleError::Rejected(Error::Other(reason)) if reason == "rejected orders"),
        "unexpected error: {error}"
    );

    runtime.shutdown();
}

#[tokio::test]
async fn guest_trap() {
    let runtime =
        boot(&sole(test_programs::MESSAGING_PRODUCE_HANDLE), Backends::defaults().await).await;
    let handler = handler(&runtime);

    // the guest asserts on `order-1`; any other payload trips its assert
    let mut message = Message::new(b"not-the-order".to_vec());
    message.topic = "orders".to_owned();
    let error = handler.handle(message).await.expect_err("a trapping guest is an error");
    assert!(matches!(error, HandleError::Trapped(_)), "unexpected error: {error}");

    runtime.shutdown();
}

// The bound on a hung handler is the deployment's guest timeout, and the
// outcome names it rather than reading as a trap.
#[tokio::test]
async fn messaging_handle_sleep() {
    const TIMEOUT: Duration = Duration::from_millis(50);

    let deployment = sole(test_programs::MESSAGING_HANDLE_SLEEP).guest_timeout(TIMEOUT);
    let runtime = boot(&deployment, Backends::defaults().await).await;
    let handler = handler(&runtime);

    let error = handler.handle(order("orders")).await.expect_err("a hung guest is an error");
    assert!(
        matches!(error, HandleError::TimedOut(timeout) if timeout == TIMEOUT),
        "unexpected error: {error}"
    );

    runtime.shutdown();
}

// A topic no route matches has nothing to run: the message is dropped with `Ok`.
#[tokio::test]
async fn no_route() {
    let runtime = boot(&routed(), Backends::defaults().await).await;
    let handler = handler(&runtime);

    handler.handle(order("nowhere")).await.expect("an unrouted topic is dropped, not an error");

    runtime.shutdown();
}

// No guest loaded at boot exports the handler: a command-only guest, or a
// sole exporter declared from a path, which is never the catch-all.
#[tokio::test]
async fn inert() {
    let command = Deployment::new().guest("guest", test_programs::MESSAGING_REQUEST_REPLY);
    let declared =
        Deployment::new().entry(GuestEntry::new("guest", test_programs::MESSAGING_PRODUCE_HANDLE));

    for deployment in [command, declared] {
        let runtime = boot(&deployment, Backends::defaults().await).await;
        let handler = MessagingHandler::new(&runtime).expect("messaging routes consistent");
        assert!(handler.is_none(), "the messaging trigger is inert");
        runtime.shutdown();
    }
}

// Two exporters with no routes are ambiguous, and a route to a guest that
// does not export the handler is refused, both as the handler is built.
#[tokio::test]
async fn inconsistent_routes() {
    let ambiguous = Deployment::new()
        .guest("a", test_programs::MESSAGING_PRODUCE_HANDLE)
        .guest("b", test_programs::MESSAGING_HANDLE_ERR);
    let command = std::fs::read(test_programs::MESSAGING_REQUEST_REPLY).expect("guest bytes");
    let misrouted =
        Deployment::new().entry(GuestEntry::new("guest", command).route_messaging("orders"));

    for (deployment, needle) in [(ambiguous, "2 capable guests"), (misrouted, "does not export")] {
        let runtime = boot(&deployment, Backends::defaults().await).await;
        let Err(error) = MessagingHandler::new(&runtime) else {
            panic!("routes refused for `{needle}` were accepted");
        };
        assert!(format!("{error:#}").contains(needle), "unexpected error: {error:#}");
        runtime.shutdown();
    }
}

// A routed guest that cannot be loaded never runs, and the outcome says so.
#[tokio::test]
async fn unloadable_guest() {
    let runtime = boot(&routed(), Backends::defaults().await).await;
    let handler = handler(&runtime);

    let error = handler.handle(order("absent")).await.expect_err("an unloadable guest is an error");
    assert!(
        matches!(&error, HandleError::Unavailable(reason) if reason.contains("absent")),
        "unexpected error: {error}"
    );

    runtime.shutdown();
}

// A consumer that returns is a server failure, not a clean exit: a backend
// that gives up on its transport must take the process down for a restart
// rather than leave it idle and looking healthy.
#[tokio::test]
async fn consumer_end() {
    let backends = Backends::defaults().await.messaging(EndedBroker);
    let runtime = boot(&sole(test_programs::MESSAGING_PRODUCE_HANDLE), backends).await;

    let error = WasiMessaging::run(&runtime).await.expect_err("an ended consumer fails the server");
    assert!(format!("{error:#}").contains("consumer ended"), "unexpected error: {error:#}");

    runtime.shutdown();
}

// The default broker's own delivery loop: every order published on the
// routed topic reaches the guest, which replies to each; the replies' own
// topic has no route, so they are dropped rather than fed back to the guest.
#[tokio::test]
async fn default_consume() {
    const ORDERS: usize = 8;

    let backends = Backends::defaults().await;
    let broker = backends.messaging.clone();
    let mut stream = broker.subscribe();
    // embedded bytes compile at boot, so the orders do not race the guest's
    // first-use load
    let guest = std::fs::read(test_programs::MESSAGING_PRODUCE_HANDLE).expect("guest bytes");
    let deployment =
        Deployment::new().entry(GuestEntry::new("guest", guest).route_messaging("orders"));
    let runtime = boot(&deployment, backends).await;
    let consumer = tokio::spawn(broker.consume(Arc::new(handler(&runtime))));

    // each order is the message the guest's own command publishes
    for _ in 0..ORDERS {
        let mut order = order("orders");
        let mut metadata = Metadata::new();
        metadata.insert("content-type".to_owned(), "text/plain".to_owned());
        metadata.insert("key".to_owned(), "customer-1".to_owned());
        metadata.insert("tenant".to_owned(), "acme".to_owned());
        order.metadata = Some(metadata);
        order.reply = Some(Reply {
            topic: "orders.reply".to_owned(),
        });
        broker.send("orders".to_owned(), order).await.expect("published");
    }

    let mut replies = 0;
    while replies < ORDERS {
        let message = next(&mut stream).await;
        if message.topic == "orders.reply" {
            assert_eq!(message.payload, b"handled");
            replies += 1;
        }
    }

    consumer.abort();
    runtime.shutdown();
}

#[tokio::test]
async fn messaging_request_reply() {
    let backends = Backends::defaults().await;
    let mut stream = backends.messaging.subscribe();

    // `backends` outlives the run so the broker stays open for the check
    // below; a closed broker ends the stream instead of staying quiet.
    run_guest(test_programs::MESSAGING_REQUEST_REPLY, backends.clone()).await;

    // Wire fidelity: the one request was published on its topic as the guest
    // sent it, and nothing else reached the broker.
    let request = next(&mut stream).await;
    assert_eq!(request.topic, "ping");
    assert_eq!(request.payload, b"ping");
    assert!(request.metadata.is_none(), "the request carried no metadata");
    assert!(
        tokio::time::timeout(Duration::from_millis(200), stream.next()).await.is_err(),
        "nothing else reached the broker"
    );
}

// fails every send with the kind its topic is named for, recording each
// topic so the run's exchanges are pinned
#[derive(Clone, Debug, Default)]
struct Faulting {
    sent: Arc<Mutex<Vec<String>>>,
}

impl MessagingClient for Faulting {
    fn endpoint(&self) -> FutureResult<Arc<dyn Endpoint>> {
        let endpoint = self.clone();
        async move { Ok(Arc::new(endpoint) as Arc<dyn Endpoint>) }.boxed()
    }
}

impl Endpoint for Faulting {
    fn consume(&self, _handler: Arc<dyn Handler>) -> FutureResult<()> {
        unreachable!("a command-only guest leaves the trigger inert")
    }

    fn send(&self, topic: String, _message: Message) -> FutureResult<()> {
        self.sent.lock().expect("sent topics").push(topic.clone());
        let fault = match topic.as_str() {
            "not-found" => Error::NotFound("not found by the suite".to_owned()).into(),
            "denied" => Error::Denied("denied by the suite".to_owned()).into(),
            "invalid" => Error::Invalid("invalid in the suite".to_owned()).into(),
            "unsupported" => Error::Unsupported("unsupported by the suite".to_owned()).into(),
            "unavailable" => Error::Unavailable("unavailable in the suite".to_owned()).into(),
            _ => anyhow::anyhow!("an untyped backend failure"),
        };
        async move { Err(fault) }.boxed()
    }

    fn request(
        &self, _topic: String, _message: Message, _options: Option<RequestOptions>,
    ) -> FutureResult<Message> {
        unreachable!("the host refuses the guest's one request")
    }
}

#[tokio::test]
async fn messaging_error_codes() {
    let backends = Backends::defaults().await.messaging(Faulting::default());

    let status = Deployment::new()
        .guest("guest", test_programs::MESSAGING_ERROR_CODES)
        .run_host::<WasiMessaging, _>(backends.clone())
        .await
        .expect("guest runs");
    assert_eq!(status, ExitStatus::SUCCESS, "guest failed");

    // the host refused every other call before it reached the broker
    let sent = backends.messaging.sent.lock().expect("sent topics").clone();
    assert_eq!(sent, ["not-found", "denied", "invalid", "unsupported", "unavailable", "other"]);
}

// The SDK settles a handler's failure by its class, and the host hands the
// backend the case the guest returned.
#[tokio::test]
async fn messaging_outcomes() {
    let runtime = boot(&sole(test_programs::MESSAGING_OUTCOMES), Backends::defaults().await).await;
    let handler = handler(&runtime);

    let error = handler.handle(order("orders")).await.expect_err("an undecodable order fails");
    assert!(
        matches!(&error, HandleError::Rejected(Error::Invalid(reason)) if reason.contains("undecodable")),
        "unexpected error: {error}"
    );
    let error = handler.handle(order("pricing")).await.expect_err("an unanswered upstream fails");
    assert!(
        matches!(&error, HandleError::Rejected(Error::Unavailable(reason)) if reason.contains("pricing")),
        "unexpected error: {error}"
    );
    let error = handler.handle(order("ledger")).await.expect_err("a refused entry fails");
    assert!(
        matches!(&error, HandleError::Rejected(Error::Other(reason)) if reason.contains("ledger")),
        "unexpected error: {error}"
    );

    runtime.shutdown();
}
