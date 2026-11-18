//! Closes the connection its argument names, with a status the peer
//! receives, then checks that no `connections` call fails for a connection
//! that is not open.

#![cfg(target_arch = "wasm32")]

use omnia_wasi_websocket::connections;
use omnia_wasi_websocket::types::{Client, CloseFrame, Message};

omnia_sdk::api::command::export!(scenario);

async fn scenario() {
    let arguments = wasip3::cli::environment::get_arguments();
    let [_, connection] = arguments.as_slice() else {
        panic!("expected `<connection>`; got {arguments:?}");
    };
    let client = Client::connect("default".to_owned()).await.expect("connect");

    // an open connection closes with the guest's status
    assert!(connections::exists(&client, connection.clone()).await.expect("exists"));
    let done = CloseFrame {
        code: 1000,
        reason: "done".to_owned(),
    };
    connections::close(&client, connection.clone(), Some(done)).await.expect("close");
    assert!(!connections::exists(&client, connection.clone()).await.expect("exists"));

    // a connection that is not open
    connections::close(&client, "gone".to_owned(), None).await.expect("close");
    let late = Message::Text("late".to_owned());
    connections::send(&client, "gone".to_owned(), late).await.expect("send");
    assert!(!connections::exists(&client, "gone".to_owned()).await.expect("exists"));
}
