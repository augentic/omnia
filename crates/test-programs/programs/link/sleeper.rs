//! Exports `omnia-test:link/ops` and sleeps well past the caller's guest
//! timeout when the message is `sleep`, so the link suite can pin the
//! wall-clock bound and confirm the target stays reachable after an abort.

#![cfg(target_arch = "wasm32")]

use std::time::Duration;

wit_bindgen::generate!({
    world: "sleeper",
    path: "wit",
});

struct Sleeper;

export!(Sleeper);

fn respond(target: &str, message: &str) -> String {
    if message == "sleep" {
        // std on `wasm32-wasip2` routes `thread::sleep` through `wasi:clocks`
        // + `wasi:io/poll`, which wasmtime-wasi serves as async host calls:
        // the callee fiber is suspended rather than spinning, so a task abort
        // lands. The test runtime does not drive the epoch, so a busy loop
        // would not be abortable here.
        std::thread::sleep(Duration::from_secs(2));
    }
    format!("{target} woke: {message}")
}

impl exports::omnia_test::link::ops::Guest for Sleeper {
    fn ping(target: String, message: String) -> String {
        respond(&target, &message)
    }

    async fn ping_async(target: String, message: String) -> String {
        respond(&target, &message)
    }
}
