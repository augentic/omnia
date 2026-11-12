//! Behaviour rows every provider of a capability meets.
//!
//! A row is one invariant of a capability trait, written once as a generic
//! `async fn` that panics when the provider breaks it. A provider's suite
//! runs every row of the traits it implements:
//!
//! - `omnia-test` runs them natively over its doubles.
//! - A guest runs them over [`WasiProvider`](crate::WasiProvider), against a
//!   real host and its backend.

pub mod state;
