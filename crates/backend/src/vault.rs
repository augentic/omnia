//! Secret vault service contract and constants.

#[cfg(not(target_arch = "wasm32"))]
mod client;

#[cfg(not(target_arch = "wasm32"))]
pub use self::client::{Locker, VaultClient};

/// The locker the `Vault` capability opens.
pub const LOCKER: &str = "locker";
