//! The level a chain root names reaches a linked callee's subscriber: the
//! root reloads its own filter to `info` and names that level on the chain,
//! reads the level back, dispatches `ping-async` to the echoer, and expects
//! the echoer to answer that it opened at `info` — from inside an INFO span
//! the host sees exported, though the echoer reloads nothing. Run with the
//! argument `unset` it names nothing and expects the echoer to open at
//! `error`; with `invalid` it names something that is not a level and expects
//! the same.

#![cfg(target_arch = "wasm32")]

wit_bindgen::generate!({
    world: "caller",
    path: "wit",
    generate_all,
});

use omnia_test::link::ops;
use tracing::level_filters::LevelFilter;

omnia_sdk::command!(scenario);

async fn scenario() {
    let arguments = wasip3::cli::environment::get_arguments();
    let expected = match arguments.get(1).map(String::as_str) {
        None => {
            omnia_wasi_otel::set_filter("info").expect("filter reloads");
            omnia_wasi_otel::set_baggage([(omnia_wasi_otel::LEVEL, "info")]);
            assert_eq!(omnia_wasi_otel::level(), LevelFilter::INFO);
            "info"
        }
        Some("unset") => {
            assert_eq!(omnia_wasi_otel::level(), LevelFilter::ERROR);
            "error"
        }
        Some("invalid") => {
            omnia_wasi_otel::set_baggage([(omnia_wasi_otel::LEVEL, "loud")]);
            assert_eq!(omnia_wasi_otel::level(), LevelFilter::ERROR);
            "error"
        }
        Some(other) => panic!("no argument, `unset`, or `invalid`; got {other:?}"),
    };

    let opened = ops::ping_async("echoer".to_owned(), String::new()).await;
    assert_eq!(opened, expected);
}
