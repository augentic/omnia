//! Every `omnia:websocket` error case reaches the guest as its own code: a
//! `BadGateway` for `unavailable`, and a `ServerError` for the rest. The
//! suite's faulting backend fails a send by the connection it names. The
//! host refuses an undeclared service and a close frame no endpoint may
//! send, and answers `unsupported` for every call the backend does not
//! serve, so none of them reaches the backend.

#![cfg(target_arch = "wasm32")]

use std::fmt::Debug;

use omnia_sdk::{WasiProvider, WebSockets as _};
use omnia_wasi_websocket::types::{Client, CloseFrame, Error, Message};
use omnia_wasi_websocket::{broadcast, connections};

omnia_sdk::api::command::export!(scenario);

async fn scenario() {
    // the backend's chosen cases, through the SDK
    for (connection, status, code) in [
        ("not-found", 500, "not-found"),
        ("denied", 500, "denied"),
        ("invalid", 500, "invalid"),
        ("unsupported", 500, "unsupported"),
        ("unavailable", 502, "unavailable"),
        ("other", 500, "other"),
    ] {
        expect_code(WasiProvider.send_to(&[connection], b"ping").await, status, code);
    }

    // the host's own rules
    assert_eq!(case(Client::connect("undeclared".to_owned()).await), "not-found");
    let client = Client::connect("default".to_owned()).await.expect("connect");
    let reserved = CloseFrame {
        code: 1005,
        reason: String::new(),
    };
    let closed = connections::close(&client, "peer".to_owned(), Some(reserved)).await;
    assert_eq!(case(closed), "invalid");
    let verbose = CloseFrame {
        code: 1000,
        reason: "x".repeat(124),
    };
    let closed = connections::close(&client, "peer".to_owned(), Some(verbose)).await;
    assert_eq!(case(closed), "invalid");

    // the calls the backend does not serve
    assert_eq!(case(connections::close(&client, "peer".to_owned(), None).await), "unsupported");
    assert_eq!(case(connections::exists(&client, "peer".to_owned()).await), "unsupported");
    let ping = Message::Text("ping".to_owned());
    let sent = broadcast::send(&client, ping, vec!["peer".to_owned()]).await;
    assert_eq!(case(sent), "unsupported");
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
