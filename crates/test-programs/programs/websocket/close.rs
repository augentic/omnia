//! Sends one message to the two connections its arguments name in a single
//! call, then closes the first with a status its peer receives. No
//! `connections` call fails for a connection that is not open.

#![cfg(target_arch = "wasm32")]

use omnia_wasi_websocket::omnia::connections;
use omnia_wasi_websocket::omnia::types::{Client, CloseFrame, Message};

omnia_sdk::api::command::export!(scenario);

async fn scenario() {
    let arguments = wasip3::cli::environment::get_arguments();
    let [_, first, second] = arguments.as_slice() else {
        panic!("expected `<connection> <connection>`; got {arguments:?}");
    };
    let client = Client::connect("default".to_owned()).await.expect("connect");

    // one message reaches both connections
    let both = Message::Text("both".to_owned());
    connections::send(&client, vec![first.clone(), second.clone()], both).await.expect("send");

    // an open connection closes with the guest's status
    assert!(connections::exists(&client, first.clone()).await.expect("exists"));
    let done = CloseFrame {
        code: 1000,
        reason: "done".to_owned(),
    };
    connections::close(&client, first.clone(), Some(done)).await.expect("close");
    assert!(!connections::exists(&client, first.clone()).await.expect("exists"));

    // a connection that is not open
    connections::close(&client, "gone".to_owned(), None).await.expect("close");
    let late = Message::Text("late".to_owned());
    connections::send(&client, vec!["gone".to_owned(), first.clone()], late).await.expect("send");
    assert!(!connections::exists(&client, "gone".to_owned()).await.expect("exists"));
}
