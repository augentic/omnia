//! Every `omnia:messaging` error case reaches the guest as its own code: a
//! `BadGateway` for `unavailable`, and a `ServerError` for the rest. The
//! suite's faulting broker fails a send by its topic. The host refuses an
//! undeclared broker, a zero timeout, and an empty reply address, so none of
//! them reaches the broker.

#![cfg(target_arch = "wasm32")]

use std::fmt::Debug;

use omnia_sdk::{Publisher as _, WasiProvider, messaging};
use omnia_wasi_messaging::omnia::request_reply;
use omnia_wasi_messaging::omnia::types::{Client, Error, Message};

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
    let unbounded = request_reply::request(&client, "orders".to_owned(), order(), Some(0)).await;
    assert_eq!(case(unbounded), "invalid");
    assert_eq!(case(request_reply::reply(&client, String::new(), order()).await), "invalid");
}

fn order() -> Message {
    Message {
        data: b"order".to_vec(),
        content_type: None,
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
