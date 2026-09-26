//! # Linking example — router guest
//!
//! Imports the host-mediated `example:link/echo` and exposes a plain `run(message)`
//! entry. The import is *not* satisfied by this component: the host has
//! polyfilled it onto the shared linker, so calling it dispatches — via the
//! runtime core's `GuestSelector` and in-memory routing — to whichever guest
//! exports `echo` (here, `responder`).
//!
//! The router names its target with the leading argument (`"responder"`), which
//! the default `FirstArgSelector` reads and forwards through.

#![cfg(target_arch = "wasm32")]

wit_bindgen::generate!({
    world: "router",
    path: "guest-link/wit",
});

struct Router;

export!(Router);

impl Guest for Router {
    // the host runs the selector, dispatches to a fresh responder, returns its result
    fn run(message: String) -> String {
        example::link::echo::echo("responder", &message)
    }

    // `echo-slow` is an async-typed import, so only an async-lifted export may call it
    async fn run_slow(message: String) -> String {
        example::link::echo::echo_slow("responder".to_owned(), message).await
    }

    // an arbitrary target: the path that reaches guests registered after startup
    fn run_to(target: String, message: String) -> String {
        example::link::echo::echo(&target, &message)
    }

    // the callee parks on a timer, so a dispatch can be in flight at deregistration
    async fn run_to_slow(target: String, message: String) -> String {
        example::link::echo::echo_slow(target, message).await
    }
}
