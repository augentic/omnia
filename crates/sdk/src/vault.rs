//! `omnia:vault`, the lockers of secrets a deployment declares.
//!
//! A guest opens a [`Locker`](locker::Locker) with [`store::open`],
//! [`DEFAULT`] unless the deployment declares another. A locker reads,
//! writes, and lists its secrets through the generated methods.

pub use crate::bindings::imports::omnia::vault::{locker, store, types};

/// The locker a guest opens when the deployment names no other.
pub const DEFAULT: &str = "default";
