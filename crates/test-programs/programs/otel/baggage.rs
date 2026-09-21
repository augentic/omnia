//! Baggage set at the chain root reaches a linked callee: the root sets one
//! entry, reads it back as the chain's current baggage, dispatches
//! `ping-async` to the echoer, and reads the entry again through its answer.
//! Run with the argument `unset` it sets nothing and expects the echoer to
//! see none.

#![cfg(target_arch = "wasm32")]

wit_bindgen::generate!({
    world: "caller",
    path: "wit",
    generate_all,
});

use omnia_test::link::ops;

omnia_sdk::command!(scenario);

async fn scenario() {
    let arguments = wasip3::cli::environment::get_arguments();
    let expected = match arguments.get(1).map(String::as_str) {
        None => {
            omnia_wasi_otel::set_baggage([("greeting", "hello, world")]);
            let baggage = omnia_wasi_otel::baggage();
            assert_eq!(baggage.get("greeting").map(|value| value.as_str()), Some("hello, world"));
            "hello, world"
        }
        Some("unset") => "",
        Some(other) => panic!("no argument or `unset`; got {other:?}"),
    };

    let answer = ops::ping_async("echoer".to_owned(), "greeting".to_owned()).await;
    assert_eq!(answer, expected);
}
