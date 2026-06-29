//! # Linking example — router guest
//!
//! Imports the host-mediated `omnia:link/echo` and exposes a plain `run(message)`
//! entry. The import is *not* satisfied by this component: the host has
//! polyfilled it onto the shared linker, so calling it dispatches — via the
//! floor's `GuestSelector` and the in-process wRPC carrier — to whichever guest
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
}
