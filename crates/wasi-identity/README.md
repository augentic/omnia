# Omnia WASI Identity

This crate provides the Identity interface for the Omnia runtime.

## Interface

Implements the `omnia:identity` WIT interface.

A guest reaches it through `omnia_sdk::Identity`. The provider's own handle asks for the identity `default`, and `named(name)` asks for another identity the host serves. A name the host does not serve fails with `no-such-identity`, which `Identity` reports as `ErrorKind::NotFound`. A backend refuses one by returning `omnia_wasi_identity::Error::NoSuchIdentity`, and any other backend error is `internal-failure`.

## Backend

- **Default**: Uses `oauth2` crate to interact with OAuth2/OIDC providers. Its one set of client credentials is the identity `default`, and it refuses every other name.
- **Stub**: `IdentityStub` issues a fixed token per identity with no identity provider, for tests and local development. It serves the identities `IDENTITY_NAMES` lists, comma-separated, and `default` alone when it is unset. Each identity's token is distinct, and every other name is refused.

## Cargo features

- `oauth` *(default)*: the `OAuth2` client-credentials default backend (`IdentityDefault`). Disable it (`default-features = false`) when supplying your own `IdentityClient` backend to skip compiling `oauth2` and its bundled HTTP/TLS stack.

## Configuration

Requires configuration via environment variables or other sources to set provider details (Client ID, Client Secret, etc.).

- **Production**: [`omnia-azure-id`](https://github.com/augentic/omnia-backends/tree/main/crates/azure-id) (Azure Managed Identity) — a one-line swap in the host, guests untouched (see the [Production Backends guide](https://github.com/augentic/omnia/blob/main/docs/guides/production-backends.md)).

## Usage

Add this crate to your `Cargo.toml` and use it in your runtime configuration:

```rust,ignore
use omnia_wasi_identity::{IdentityDefault, WasiIdentity};

omnia::runtime!({
    hosts: {
        WasiIdentity: IdentityDefault,
    }
});
```

## License

MIT OR Apache-2.0
