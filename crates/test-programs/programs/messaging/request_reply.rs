//! One `wasi:messaging/request-reply` round trip from a command guest: a
//! request carrying options returns the default backend's canned
//! acknowledgement, and replying to a message that names no reply topic is
//! a no-op.

#![cfg(target_arch = "wasm32")]

use omnia_wasi_messaging::request_reply::{self, RequestOptions};
use omnia_wasi_messaging::types::{Client, Message};

omnia_guest::command!(scenario);

async fn scenario() {
    let client = Client::connect("default".to_owned()).await.expect("connect");

    let options = RequestOptions::new();
    options.set_timeout_ms(1_000);
    options.set_expected_replies(1);

    let request = Message::new(b"ping");
    request.set_content_type("text/plain");
    let replies = request_reply::request(&client, "ping".to_owned(), &request, Some(options))
        .await
        .expect("request");

    assert_eq!(replies.len(), 1);
    let reply = &replies[0];
    assert_eq!(reply.topic().as_deref(), Some("response"));
    assert_eq!(reply.data(), b"ACK");
    assert_eq!(reply.metadata(), None);

    // The request was never received, so it names no reply topic.
    request_reply::reply(&request, Message::new(b"ignored")).await.expect("reply is a no-op");
}
