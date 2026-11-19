//! `omnia:vault`, the lockers of secrets a deployment declares.
//!
//! A guest opens a [`Locker`] with [`open`], [`DEFAULT`] unless the
//! deployment declares another. A locker reads, writes, and lists its secrets
//! through the generated methods.

use crate::Result;
pub use crate::bindings::imports::omnia::vault::{locker, store, types};
pub use crate::vault::locker::Locker;

/// The locker a guest opens when the deployment names no other.
pub const DEFAULT: &str = "default";

/// Open the locker `name`, a binding the deployment declares.
///
/// # Errors
///
/// The host's failure, under its code: `not-found` for a locker the
/// deployment does not declare.
pub async fn open(name: &str) -> Result<Locker> {
    Ok(store::open(name.to_owned()).await?)
}
