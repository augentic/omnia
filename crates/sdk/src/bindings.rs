//! The guest bindings `wit-bindgen` generates from every `omnia:*` package.
//!
//! `mise run bindings` regenerates them, and `mise run bindings-check` fails
//! when they drift from the WIT. Nothing here is edited by hand.

#![allow(
    missing_docs,
    trivial_numeric_casts,
    unsafe_code,
    unsafe_op_in_unsafe_fn,
    clippy::all,
    clippy::cargo,
    clippy::nursery,
    clippy::pedantic,
    clippy::allow_attributes_without_reason,
    reason = "wit-bindgen output"
)]

#[rustfmt::skip]
pub mod imports;
#[rustfmt::skip]
pub mod messaging;
#[rustfmt::skip]
pub mod websocket;
