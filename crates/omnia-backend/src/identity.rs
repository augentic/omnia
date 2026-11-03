//! Identity and authentication service contract, value types, and default implementations.

#[cfg(not(target_arch = "wasm32"))]
mod client;
#[cfg(all(not(target_arch = "wasm32"), feature = "oauth"))]
mod oauth;

#[cfg(not(target_arch = "wasm32"))]
pub use self::client::{Identity, IdentityClient, IdentityStub};
#[cfg(all(not(target_arch = "wasm32"), feature = "oauth"))]
pub use self::oauth::{ConnectOptions, IdentityDefault};

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
