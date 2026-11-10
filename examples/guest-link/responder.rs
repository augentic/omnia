//! # Linking example — responder guest
//!
//! Exports `example:link/echo`. It declares no HTTP/messaging trigger, so the host
//! never routes inbound traffic to it directly: it is reached *only* when another
//! guest's host-mediated import is dispatched here (the `router` calls it).
//!
//! The host instantiates this guest fresh for every dispatched call
//! (instance-per-call) and discards it afterwards.

#![cfg(target_arch = "wasm32")]

// `generate_all` also generates the `wasi:clocks` import bindings so the
// `echo-slow` await is driven by this component's own async runtime.
wit_bindgen::generate!({
    world: "responder",
    path: "guest-link/wit",
    generate_all,
});

struct Responder;

export!(Responder);

impl exports::example::link::echo::Guest for Responder {
    // `target` is the selector's identity, forwarded unchanged
    fn echo(target: String, message: String) -> String {
        format!("{target} echoes: {message}")
    }

    // parks on a real host timer, so the round-trip completes against a pending callee
    async fn echo_slow(target: String, message: String) -> String {
        wasi::clocks::monotonic_clock::wait_for(5_000_000).await; // 5ms
        format!("{target} echoes slowly: {message}")
    }
}
