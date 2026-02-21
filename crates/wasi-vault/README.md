# Omnia WASI Vault

This crate provides the Secrets Vault interface for the Omnia runtime.

## Interface

Implements the `wasi:vault` WIT interface.

## Backend

- **Default**: In-memory implementation. Secrets are not persisted.

## Usage

Add this crate to your `Cargo.toml` and use it in your runtime configuration:

```rust,ignore
use omnia::runtime;
use omnia_wasi_vault::VaultDefault;

omnia::runtime!({
    "vault": VaultDefault,
});
```

## License

MIT OR Apache-2.0
