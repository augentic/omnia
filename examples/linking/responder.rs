//! # Linking example — responder guest
//!
//! Exports `omnia:link/echo`. It declares no HTTP/messaging trigger, so the host
//! never routes inbound traffic to it directly: it is reached *only* when another
//! guest's host-mediated import is dispatched here (the `router` calls it).
//!
//! The host instantiates this guest fresh for every dispatched call
//! (instance-per-call) and discards it afterwards.

#![cfg(target_arch = "wasm32")]

wit_bindgen::generate!({
    world: "responder",
    path: "linking/wit",
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
}
