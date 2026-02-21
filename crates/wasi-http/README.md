# Omnia WASI HTTP

This crate provides the HTTP interface for the Omnia runtime.

## Interface

Implements the `wasi:http` WIT interface (WASI Preview 2).

## Backend

Uses `hyper` and `axum` to handle outgoing requests and incoming server connections.

## Usage

Add this crate to your `Cargo.toml` and use it in your runtime configuration:

```rust,ignore
use omnia::runtime;
use omnia_wasi_http::WasiHttpCtx;

omnia::runtime!({
    "http": WasiHttpCtx,
});
```

## License

MIT OR Apache-2.0
