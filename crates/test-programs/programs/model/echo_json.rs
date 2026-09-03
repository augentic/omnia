//! The echo default shapes a `format::json` answer as an object.

#![cfg(target_arch = "wasm32")]

use omnia_guest::model::{Format, Model as _, Request, WasiModel};
use test_programs::user;

omnia_guest::command!(scenario);

async fn scenario() {
    let reply = WasiModel
        .complete(Request::builder().messages(vec![user("hi")]).format(Format::Json).build())
        .await
        .expect("echo answers json");
    assert_eq!(reply.answer, r#"{"echo":"hi"}"#);
}
