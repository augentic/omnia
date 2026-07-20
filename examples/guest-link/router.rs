//! # Linking example — router guest
//!
//! Imports the host-mediated `omnia:link/echo` and exposes a plain `run(message)`
//! entry. The import is *not* satisfied by this component: the host has
//! polyfilled it onto the shared linker, so calling it dispatches — via the
//! runtime core's `GuestSelector` and the in-process wRPC carrier — to whichever guest
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
    /// Call the host-mediated `echo`, naming `responder` as the target. The host
    /// runs the selector, dispatches to the responder (instantiated fresh), and
    /// returns its typed result.
    fn run(message: String) -> String {
        omnia::link::echo::echo("responder", &message)
    }

    /// The async-lifted dual: `echo-slow` is an async-typed import, so only an
    /// async-lifted export may call it.
    async fn run_slow(message: String) -> String {
        omnia::link::echo::echo_slow("responder".to_owned(), message).await
    }

    /// Call the host-mediated `echo` naming an arbitrary target — the path that
    /// reaches guests registered after startup (dynamic registration).
    fn run_to(target: String, message: String) -> String {
        omnia::link::echo::echo(&target, &message)
    }

    /// The arbitrary-target dual of `run-slow`: an async-lifted call whose
    /// callee parks on a timer, so a dispatch can be genuinely in flight when
    /// the target is deregistered.
    async fn run_to_slow(target: String, message: String) -> String {
        omnia::link::echo::echo_slow(target, message).await
    }
}
