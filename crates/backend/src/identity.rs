//! Identity and authentication service contract and value types.

#[cfg(not(target_arch = "wasm32"))]
mod client;

#[cfg(not(target_arch = "wasm32"))]
pub use self::client::{Identity, IdentityClient};

/// An `OAuth` 2.0 access token along with its expiration time.
///
/// Mirrors the `access-token` record from `omnia:identity/credentials`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccessToken {
    /// The `OAuth` 2.0 access token.
    pub token: String,
    /// Seconds until this token expires.
    pub expires_in: u64,
}
