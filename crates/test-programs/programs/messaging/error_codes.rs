//! Every `omnia:messaging` error case reaches the guest as its own code: a
//! `BadGateway` for `unavailable`, and a `ServerError` for the rest. The
//! suite's faulting broker fails a send by its topic. The host refuses an
//! undeclared broker, a zero timeout, and an empty reply address, so none of
//! them reaches the broker.

#![cfg(target_arch = "wasm32")]

use std::fmt::Debug;

use omnia_sdk::messaging::types::{Client, Error, Message};
use omnia_sdk::messaging::{DEFAULT, producer, request_reply};

omnia_sdk::command::export!(scenario);

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
        expect_code(publish(topic).await, status, code);
    }

    // the host's own rules
    assert_eq!(case(Client::connect("undeclared".to_owned()).await), "not-found");
    let client = Client::connect("default".to_owned()).await.expect("connect");
    let unbounded = request_reply::request(&client, "orders".to_owned(), order(), Some(0)).await;
    assert_eq!(case(unbounded), "invalid");
    assert_eq!(case(request_reply::reply(&client, String::new(), order()).await), "invalid");
}

async fn publish(topic: &str) -> omnia_sdk::Result<()> {
    let client = Client::connect(DEFAULT.to_owned()).await?;
    producer::send(&client, topic.to_owned(), order()).await?;
    Ok(())
}

fn order() -> Message {
    Message::new(b"order")
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
