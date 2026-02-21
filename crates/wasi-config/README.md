# Omnia WASI Config

This crate provides the Config interface for the Omnia runtime.

## Interface

Implements the `wasi:config` WIT interface.

## Backend

- **Default**: Wraps `wasmtime-wasi-config` to provide configuration values from the host environment or configuration files.

## Usage

Add this crate to your `Cargo.toml` and use it in your runtime configuration:

```rust,ignore
use omnia::runtime;
use omnia_wasi_config::ConfigDefault;

omnia::runtime!({
    "config": ConfigDefault,
});
```

## License

MIT OR Apache-2.0
