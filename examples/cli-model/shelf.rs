//! # Model example — `shelf` guest (Phase 2a)
//!
//! Exports `omnia:model/references`. It declares no HTTP/messaging trigger, so
//! the host never routes inbound traffic to it: it is reached *only* when a
//! backend's host-mediated `resolve` lands here (a `complete` prompt that set
//! `grants.references = "shelf"`).
//!
//! The host instantiates this guest fresh for every dispatched `resolve` call
//! (instance-per-call) and discards it afterwards, so it holds no state and can
//! never re-enter the guest that called `complete`.

#![cfg(target_arch = "wasm32")]

use omnia_wasi_model::shelf::exports::omnia::model::references::Guest;

struct Shelf;

omnia_wasi_model::shelf::export!(Shelf with_types_in omnia_wasi_model::shelf);

impl Guest for Shelf {
    /// Resolve a reference to its bytes. A deterministic transform so the
    /// acceptance test can prove the bytes round-trip through the host→guest
    /// seam: `"alpha"` resolves to `b"shelf:alpha"`.
    fn resolve(reference: String) -> Vec<u8> {
        format!("shelf:{reference}").into_bytes()
    }
}
