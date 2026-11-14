# Omnia WASI Vault

This crate provides the Secrets Vault interface for the Omnia runtime.

## Interface

Implements the `omnia:vault` WIT interface.

A guest reaches it through `omnia_sdk::SecretStore`. The provider's own handle opens the locker `default`, and `named(name)` opens another locker. `keys(after)` lists one locker in key order, after the key it is given.

## Backend

- **Default**: In-memory implementation. Secrets are not persisted, and a locker opens on first use.

- **Production**: [`omnia-azure-vault`](https://github.com/augentic/omnia-backends/tree/main/crates/azure-vault) (Azure Key Vault) — a one-line swap in the host, guests untouched (see the [Production Backends guide](https://github.com/augentic/omnia/blob/main/docs/guides/production-backends.md)).

## Usage

Add this crate to your `Cargo.toml` and use it in your runtime configuration:

```rust,ignore
use omnia_wasi_vault::{VaultDefault, WasiVault};

omnia::runtime!({
    hosts: {
        WasiVault: VaultDefault,
    }
});
```

## License

MIT OR Apache-2.0
