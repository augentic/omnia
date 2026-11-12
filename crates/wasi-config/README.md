# Omnia WASI Config

This crate provides the Config interface for the Omnia runtime.

## Interface

Implements the `wasi:config` WIT interface.

A guest reaches it through `omnia_sdk::Config`. `get` answers `None` for a key the host does not set, and `ConfigExt::require` fails one with `ErrorKind::NotFound`.

## Backend

- **Default**: Wraps `wasmtime-wasi-config` to provide configuration values from the host environment or configuration files.

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
