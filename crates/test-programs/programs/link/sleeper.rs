//! Exports `omnia-test:link/ops` and sleeps well past the caller's guest
//! timeout when `ping-async` gets `sleep`, so the link suite can pin the
//! wall-clock bound and confirm the target stays reachable after an abort.

#![cfg(target_arch = "wasm32")]

use std::time::Duration;

wit_bindgen::generate!({
    world: "sleeper",
    path: "wit",
});

struct Sleeper;

export!(Sleeper);

impl exports::omnia_test::link::ops::Guest for Sleeper {
    fn ping(target: String, message: String) -> String {
        format!("{target} woke: {message}")
    }

    async fn ping_async(target: String, message: String) -> String {
        if message == "sleep" {
            // `thread::sleep` waits on WASI 0.3's async `wait-for`, which only an
            // async-typed export may block on, so the fiber suspends and an abort
            // lands; a busy loop would not be abortable without the epoch
            std::thread::sleep(Duration::from_secs(2));
        }
        format!("{target} woke: {message}")
    }
}
