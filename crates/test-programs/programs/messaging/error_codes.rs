//! Every `omnia:messaging` error case reaches the guest as its own code: a
//! `BadGateway` for `unavailable`, and a `ServerError` for the rest. The
//! suite's faulting broker fails a send by its topic. The host refuses an
//! undeclared broker, a message id, a zero timeout, an empty reply address,
//! and every interface it does not serve, so none of them reaches the broker.

#![cfg(target_arch = "wasm32")]

use std::fmt::Debug;

use omnia_sdk::{Publisher as _, WasiProvider, messaging};
use omnia_wasi_messaging::types::{Client, Error, Message};
use omnia_wasi_messaging::{admin, consumer, log, producer, request_reply};

omnia_sdk::api::command::export!(scenario);

async fn scenario() {
    // the backend's chosen cases, through the SDK
    for (topic, status, code) in [
        ("not-found", 500, "not-found"),
        ("denied", 500, "denied"),
        ("invalid", 500, "invalid"),
        ("unsupported", 500, "unsupported"),
        ("unavailable", 502, "unavailable"),
        ("other", 500, "other"),
    ] {
        let message = messaging::Message::new(b"order");
        expect_code(WasiProvider.publish(topic, &message).await, status, code);
    }

    // the host's own rules
    assert_eq!(case(Client::connect("undeclared".to_owned()).await), "not-found");
    let client = Client::connect("default".to_owned()).await.expect("connect");
    let identified = Message {
        id: Some("order-1".to_owned()),
        ..order()
    };
    assert_eq!(case(producer::send(&client, "orders".to_owned(), identified).await), "unsupported");
    let unbounded = request_reply::request(&client, "orders".to_owned(), order(), Some(0)).await;
    assert_eq!(case(unbounded), "invalid");
    assert_eq!(case(request_reply::reply(String::new(), order()).await), "invalid");

    // the interfaces the host does not serve
    let received = consumer::receive(&client, "orders".to_owned(), 1, 0).await;
    assert_eq!(case(received), "unsupported");
    let appended = log::append(&client, "orders".to_owned(), order(), None).await;
    assert_eq!(case(appended), "unsupported");
    let read = log::read(&client, "orders".to_owned(), log::Start::First, None).await;
    assert_eq!(case(read), "unsupported");
    assert_eq!(case(admin::list_topics(&client, ">".to_owned(), None, None).await), "unsupported");
    assert_eq!(case(admin::purge_topics(&client, ">".to_owned()).await), "unsupported");
}

fn order() -> Message {
    Message {
        data: b"order".to_vec(),
        content_type: None,
        id: None,
        key: None,
        metadata: Vec::new(),
    }
}

fn expect_code<T: Debug>(result: omnia_sdk::Result<T>, status: u16, code: &str) {
    match result {
        Ok(value) => panic!("returned {value:?}, not a `{code}` failure"),
        Err(err) => {
            assert_eq!((err.status().as_u16(), err.code()), (status, code), "{err}");
        }
    }
}

fn case<T: Debug>(result: Result<T, Error>) -> &'static str {
    match result.expect_err("the host refuses the call") {
        Error::NotFound(_) => "not-found",
        Error::Denied(_) => "denied",
        Error::Invalid(_) => "invalid",
        Error::Unsupported(_) => "unsupported",
        Error::Unavailable(_) => "unavailable",
        Error::Other(_) => "other",
    }
}
