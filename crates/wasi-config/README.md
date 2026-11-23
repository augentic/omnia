# Omnia WASI Config

This crate provides the Config interface for the Omnia runtime.

## Interface

Implements `omnia:config`, the interface the SDK's `config` module binds, over one
`ConfigClient` per backend.

- A guest reads a key through `store.get`. An unset key is `none`, not an
  error, and a key may be set to the empty string. `config::require`
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

- **Default**: Serves the host variables `CONFIG_KEYS` names, comma-separated, from a snapshot taken when it connects. It serves none when `CONFIG_KEYS` is unset.
  - Each entry is a whole name, or a prefix that ends in `*`, as in `[env] inherit`. `*` alone serves every variable.
  - A `get` for any other key answers unset. The first one for a variable the host sets logs a warning naming the variable.
  - It skips a variable whose name or value is not Unicode.
  - It lists every variable it serves through `entries`.

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

The runtime above serves the variables `CONFIG_KEYS` names when it starts. A runtime can compile its keys in instead, and then never reads `CONFIG_KEYS`:

```rust,ignore
use omnia_wasi_config::{ConfigDefault, ConnectOptions, WasiConfig};

omnia::runtime!({
    hosts: {
        WasiConfig: ConfigDefault(ConnectOptions {
            keys: vec!["UPSTREAM_URL".to_owned()],
        }),
    }
});
```

To let a `CONFIG_KEYS` that is set replace the compiled-in keys, load it over them with `with_env`. A malformed `CONFIG_KEYS` then fails startup:

```rust,ignore
use omnia_wasi_config::{ConfigDefault, ConnectOptions, WasiConfig};

omnia::runtime!({
    hosts: {
        WasiConfig: ConfigDefault(ConnectOptions {
            keys: vec!["UPSTREAM_URL".to_owned()],
        }.with_env()?),
    }
});
```

## License

MIT OR Apache-2.0
