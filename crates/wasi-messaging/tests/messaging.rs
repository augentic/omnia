//! End-to-end tests for `wasi:messaging`: every scenario runs a real guest
//! component from `crates/test-programs` through the omnia runtime over the
//! in-memory broker. Command exports are driven through `wasi:cli/run`; the
//! `incoming-handler` export is driven in-process through
//! [`MessagingHandler`], bypassing the server loop. The guest asserts what it
//! observes across the boundary (and traps on failure); the host side asserts
//! what reached the broker, from a subscription opened before the guest runs.

#![cfg(not(target_arch = "wasm32"))]

use std::time::Duration;

use futures::StreamExt as _;
use omnia::ExitStatus;
use omnia_test::host::{Backends, Deployment};
use omnia_wasi_messaging::{
    Client as _, Message, MessagingHandler, Reply, Subscriptions, WasiMessaging,
};
use omnia_wasi_otel::WasiOtel;

// Every guest program in `crates/test-programs` must have a matching test
// here; a new program without one fails to compile.
test_programs::foreach_messaging!();

/// Run one command guest program against `backends`, requiring a clean exit.
async fn run_guest(wasm: &str, backends: Backends) {
    let status = Deployment::new()
        .guest("guest", wasm)
        .run_host::<WasiMessaging, _>(backends)
        .await
        .expect("guest runs");
    assert_eq!(status, ExitStatus::SUCCESS, "guest `{wasm}` failed");
}

/// The next message the broker fans out to `stream`, within a bounded wait.
async fn next(stream: &mut Subscriptions) -> Message {
    tokio::time::timeout(Duration::from_secs(5), stream.next())
        .await
        .expect("a message reaches the subscription")
        .expect("the broker stays open")
}

/// A message's metadata as sorted pairs.
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
    // asserts its fields and replies on that topic.
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
