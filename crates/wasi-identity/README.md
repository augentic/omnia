# Omnia WASI Identity

This crate provides the Identity interface for the Omnia runtime.

## Interface

Implements the `wasi:identity` WIT interface.

## Backend

- **Default**: Uses `oauth2` crate to interact with OAuth2/OIDC providers.

## Configuration

Requires configuration via environment variables or other sources to set provider details (Client ID, Client Secret, etc.).

## Usage

Add this crate to your `Cargo.toml` and use it in your runtime configuration:

```rust,ignore
use omnia::runtime;
use omnia_wasi_identity::IdentityDefault;

omnia::runtime!({
    "identity": IdentityDefault,
});
```

## License

MIT OR Apache-2.0
