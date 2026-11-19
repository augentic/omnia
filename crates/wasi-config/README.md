# Omnia WASI Config

This crate provides the Config interface for the Omnia runtime.

## Interface

Implements `omnia:config`, the interface the SDK's `Config` calls, over one
`ConfigClient` per backend.

- A guest reads a key through `store.get`. An unset key is `none`, not an
  error, and a key may be set to the empty string. `ConfigExt::require`
  fails an unset key with the code `not-found`.
- Every backend reads a portable key, and may reject any other with
  `invalid`. A portable key:
  - is 1 to 253 bytes long;
  - holds only uppercase ASCII letters, digits, and `_`;
  - does not start with a digit.
- A deployment may change a value at any time, so two reads of one key may
  differ.
- `entries` is optional, and pages the entries under a key prefix. A
  backend that does not override `ConfigClient::entries` answers
  `unsupported`.
- A backend picks a failure's case by returning an
  `omnia_wasi_config::Error`, and any other failure is `other`.
- The host itself answers `unsupported` for nothing.

## Backend

- **Default**: Answers from a snapshot of the host process environment, taken when it connects. It skips a variable whose name or value is not Unicode, and lists every variable through `entries`.

## Usage

Add this crate to your `Cargo.toml` and use it in your runtime configuration:

```rust,ignore
use omnia_wasi_config::{ConfigDefault, WasiConfig};

omnia::runtime!({
    hosts: {
        WasiConfig: ConfigDefault,
    }
});
```

## License

MIT OR Apache-2.0
