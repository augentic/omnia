# Omnia WASI WebSocket

This crate provides the WebSocket interface for the Omnia runtime.

## Interface

Implements `omnia:websocket`, the interface the SDK's `WebSockets` calls,
over one `Endpoint` per backend.

- A guest connects to a WebSocket service by name. The deployment declares
  one, `default`, and any other name fails with `not-found`.
- Each event names the connection that sent it by an id the backend mints.
  The id is opaque, and no other connection ever gets it.
- A message is text or binary, and travels in a frame of that type. Every
  backend carries a portable message and close frame, and may reject any
  other with `invalid`.
- `connections` sends to the connections a guest names, closes one, and
  asks whether one is open. A send skips a connection that is not open,
  and closing one succeeds. A close code no endpoint may send is `invalid`, and so is a
  reason over 123 bytes.
- `broadcast` is optional, and sends to every open connection except those
  the guest names.
- A receiving guest exports `incoming-handler`. The host routes each event
  to a guest, and sets its `deadline` to when it stops waiting. An error leaves the connection open.
- The host answers `unsupported` for what a backend's `Endpoint` does not
  serve yet:
  - `close` and `exists`;
  - a `broadcast` that leaves out a connection.
- An event carries no backend metadata yet.

## Backend

- **Default**: A `tokio-tungstenite` server that external clients connect to. It serves every call, and mints each connection a random id.

## Usage

Add this crate to your `Cargo.toml` and use it in your runtime configuration:

```rust,ignore
use omnia_wasi_websocket::{WasiWebSocket, WebSocketDefault};

omnia::runtime!({
    hosts: {
        WasiWebSocket: WebSocketDefault,
    }
});
```

## License

MIT OR Apache-2.0
