# Omnia WASI HTTP

This crate provides the HTTP interface for the Omnia runtime.

## Interface

Implements the `wasi:http` WIT interface (WASI Preview 3).

## Backend

Uses `hyper` and `axum` to handle outgoing requests and incoming server connections.

## Usage

Add this crate to your `Cargo.toml` and use it in your runtime configuration:

```rust,ignore
use omnia_wasi_http::{HttpDefault, WasiHttp};

omnia::runtime!({
    hosts: {
        WasiHttp: HttpDefault,
    }
});
```

## License

MIT OR Apache-2.0
