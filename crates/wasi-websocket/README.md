# Omnia WASI WebSocket

This crate provides the WebSocket interface for the Omnia runtime.

## Interface

Implements the `wasi:websocket` WIT interface.

## Backend

Uses `tokio-tungstenite` to handle WebSocket connections.

## Usage

Add this crate to your `Cargo.toml` and use it in your runtime configuration:

```rust,ignore
use omnia::runtime;
use omnia_wasi_websocket::WebsocketDefault;

omnia::runtime!({
    "websocket": WebsocketDefault,
});
```

## License

MIT OR Apache-2.0
