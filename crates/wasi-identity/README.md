# Omnia WASI Identity

This crate provides the Identity interface for the Omnia runtime.

## Interface

Implements `omnia:identity`, the interface the SDK's `Identity` calls, over
one `IdentityClient` per backend.

- A guest opens an identity by name and asks for tokens through the handle
  `open` returns. A name the deployment does not declare fails with
  `not-found`. The SDK's unnamed handle opens `default`.
- Every backend accepts a portable name and scope, and may reject any other
  with `invalid`:
  - a name is not empty and holds only ASCII letters, digits, `_`, and `-`;
  - a scope is an `OAuth` 2.0 scope token, one or more printable ASCII
    characters other than space, `"`, and `\`.
- An empty `scopes` asks for the identity's default scopes. A portable
  request names one scope, and a backend may fail any other number with
  `unsupported`.
- A token is an `OAuth` 2.0 bearer token. Its `expires-at` is in
  milliseconds since the Unix epoch, and `Identity` reports it as the time
  the token has left.
- A backend picks a failure's case by returning an
  `omnia_wasi_identity::Error`, and any other failure is `other`.
- The host itself answers `unsupported` for nothing.

## Backend

- **Default**: Uses `oauth2` crate to interact with OAuth2/OIDC providers. Its one set of client credentials is the identity `default`, and it refuses every other name with `not-found`. It caches a token per set of scopes and reports the time a cached token has left. An issuer that states no expiry is taken to issue tokens that last an hour, and any failure to get a token from the issuer is `other`.
- **Stub**: `IdentityStub` issues a fixed token per identity with no identity provider, for tests and local development. It serves the identities `IDENTITY_NAMES` lists, comma-separated, and `default` alone when it is unset. Each identity's token is distinct and lasts an hour, and every other name is refused with `not-found`.

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
