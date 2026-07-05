//! # Linking example — responder guest
//!
//! Exports `omnia:link/echo`. It declares no HTTP/messaging trigger, so the host
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

impl exports::omnia::link::echo::Guest for Responder {
    /// Echo the message back, tagged with the identity the caller selected. The
    /// `target` argument is the selector's identity, forwarded through unchanged;
    /// the responder simply proves the round-trip by echoing it.
    fn echo(target: String, message: String) -> String {
        format!("{target} echoes: {message}")
    }

    /// The async-lifted dual: park on a real host timer before answering, so the
    /// dispatch round-trip completes against a callee that was genuinely pending.
    async fn echo_slow(target: String, message: String) -> String {
        wasi::clocks::monotonic_clock::wait_for(5_000_000).await; // 5ms
        format!("{target} echoes slowly: {message}")
    }
}
