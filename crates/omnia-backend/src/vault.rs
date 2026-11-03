//! Secret vault service contract, constants, and default implementation.

#[cfg(not(target_arch = "wasm32"))]
mod client;
#[cfg(all(not(target_arch = "wasm32"), feature = "vault-default"))]
mod default;

#[cfg(not(target_arch = "wasm32"))]
pub use self::client::{Locker, VaultClient};
#[cfg(all(not(target_arch = "wasm32"), feature = "vault-default"))]
pub use self::default::VaultDefault;

/// The locker the `Vault` capability opens.
pub const LOCKER: &str = "locker";
