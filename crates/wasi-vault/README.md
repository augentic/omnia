# Omnia WASI Vault

This crate provides the Secrets Vault interface for the Omnia runtime.

## Interface

Implements `omnia:vault`, the interface the SDK's `vault` module binds, over
one `Locker` per backend.

- A guest opens a locker by name and calls it through the handle `open`
  returns. A name the deployment does not declare fails with `not-found`.
  A guest that names no other opens `default`. An absent secret is `none`, not
  an error.
- Every backend stores a portable locker name, secret name, and value, and
  may reject any other with `invalid`:
  - a locker name is not empty and holds only ASCII letters, digits, `_`,
    and `-`;
  - a secret name is 1 to 63 characters long and holds only lowercase ASCII
    letters, digits, and `-`;
  - a value is not empty and at most 16 kibibytes.
- A `set` replaces any value there. Deleting an absent secret succeeds, and
  a deleted name may be set again at once.
- `list-secrets` pages the secret names under a prefix, in no particular
  order. An empty prefix pages a whole locker.
- A backend picks a failure's case by returning an
  `omnia_wasi_vault::Error`, and any other failure is `other`.
- The host itself answers `unsupported` for nothing.

## Backend

- **Default**: In-memory implementation. Secrets are not persisted. It serves the lockers
  `VAULT_LOCKERS` declares (comma-separated, default `default`).

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
