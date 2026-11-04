use omnia_backend::identity::Identity;

/// Represents an identity resource in the WASI Identity host.
pub type IdentityProxy = omnia_core::Proxy<dyn Identity>;
