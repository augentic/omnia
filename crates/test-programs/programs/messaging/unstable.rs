//! Raw bindings: the unstable `consumer`, `log`, and `admin` interfaces
//! link, and the in-memory default answers each call with `unsupported`.

#![cfg(target_arch = "wasm32")]

use omnia_sdk::messaging::types::{Client, Error, Message};
use omnia_sdk::messaging::{DEFAULT, admin, consumer, log};

omnia_sdk::command::export!(scenario);

async fn scenario() {
    let client = Client::connect(DEFAULT.to_owned()).await.expect("connect");

    let received = consumer::receive(&client, "orders".to_owned(), 1, 0).await;
    unsupported(received.err(), "no consumer");

    let appended = log::append(&client, "orders".to_owned(), Message::new(b"order"), None).await;
    unsupported(appended.err(), "no log");
    let read = log::read(&client, "orders".to_owned(), log::Start::First, None).await;
    unsupported(read.err(), "no log");

    let listed = admin::list_topics(&client, "orders".to_owned(), None, None).await;
    unsupported(listed.err(), "no topic administration");
    let purged = admin::purge_topics(&client, "orders".to_owned()).await;
    unsupported(purged.err(), "no topic administration");
}

#[track_caller]
fn unsupported(error: Option<Error>, needle: &str) {
    assert!(
        matches!(&error, Some(Error::Unsupported(detail)) if detail.contains(needle)),
        "expected `unsupported` holding {needle:?}, got {error:?}"
    );
}
