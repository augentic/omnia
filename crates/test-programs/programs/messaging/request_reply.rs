//! One `omnia:messaging/request-reply` round trip from a command guest: a
//! request bounded by a timeout returns the default backend's canned
//! acknowledgement as its one reply.

#![cfg(target_arch = "wasm32")]

use omnia_sdk::messaging::request_reply;
use omnia_sdk::messaging::types::{Client, Message};

omnia_sdk::api::command::export!(scenario);

async fn scenario() {
    let client = Client::connect("default".to_owned()).await.expect("connect");

    let request = Message {
        data: b"ping".to_vec(),
        content_type: None,
        key: None,
        metadata: Vec::new(),
    };
    let reply = request_reply::request(&client, "ping".to_owned(), request, Some(1_000))
        .await
        .expect("request");

    assert_eq!(reply.data, b"ACK");
    assert_eq!((reply.content_type, reply.key), (None, None));
    assert!(reply.metadata.is_empty(), "the acknowledgement carries no metadata");
}
