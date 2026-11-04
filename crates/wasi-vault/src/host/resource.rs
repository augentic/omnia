use omnia_backend::vault::Locker;

/// Represents a locker resource in the WASI Vault.
pub type LockerProxy = omnia_core::Proxy<dyn Locker>;
