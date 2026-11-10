# Omnia WASI Key-Value

This crate provides the Key-Value interface for the Omnia runtime.

## Interface

Implements the `wasi:keyvalue` WIT interface.

## Backend

- **Default**: In-memory cache using `moka`. Data is not persisted across restarts.

- **Production**: [`omnia-redis`](https://github.com/augentic/omnia-backends/tree/main/crates/redis) (Redis), [`omnia-nats`](https://github.com/augentic/omnia-backends/tree/main/crates/nats) (NATS JetStream) — a one-line swap in the host, guests untouched (see the [Production Backends guide](https://github.com/augentic/omnia/blob/main/docs/guides/production-backends.md)).

## Faces

One crate name serves three builds:

- **Guest**, on `wasm32`: the `wasi:keyvalue` and `omnia:keyvalue` bindings.
- **Contract**, on native targets: `KeyValueClient`, `Bucket`, `add_i64`, and the `WasiKeyValue` marker. It depends on no engine crate, so a backend crate implements the contract without building `wasmtime`.
- **Host**, behind the `host` feature, on by default: `Host` for `WasiKeyValue` and the in-memory `KeyValueDefault`.

`BUCKET`, the bucket `StateStore` opens, is defined on every target.

A backend crate takes the contract alone:

```toml
omnia-wasi-keyvalue = { version = "0.38", default-features = false }
```

## Usage

Add this crate to your `Cargo.toml` and use it in your runtime configuration:

```rust,ignore
use omnia_wasi_keyvalue::{KeyValueDefault, WasiKeyValue};

omnia::runtime!({
    hosts: {
        WasiKeyValue: KeyValueDefault,
    }
});
```

Or in the list form, where the backend names `WasiKeyValue` itself:

```rust,ignore
use omnia_wasi_keyvalue::KeyValueDefault;

omnia::runtime!({
    hosts: [KeyValueDefault]
});
```

## License

MIT OR Apache-2.0
