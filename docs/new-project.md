# Starting a New Project

[Getting Started](getting-started.md) runs the examples inside this repository. This page sets up a **standalone project**: your own workspace with a guest crate and a host crate, depending on the published `omnia-*` crates.

## Prerequisites

- **Rust 1.99 or later** with the `wasm32-wasip2` target. Pin both in a `rust-toolchain.toml` at your workspace root so every checkout gets them automatically:

```toml
[toolchain]
channel = "stable"
targets = ["wasm32-wasip2"]
```

## Workspace layout

Two crates — the guest (compiles to `.wasm`) and the host (a native binary):

```text
my-app/
├── Cargo.toml          # workspace
├── rust-toolchain.toml
├── guest/
│   ├── Cargo.toml
│   └── src/lib.rs
└── host/
    ├── Cargo.toml
    └── src/main.rs
```

```toml
# Cargo.toml (workspace root)
[workspace]
resolver = "3"
members = ["guest", "host"]
```

## The guest crate

A guest is a library crate with `crate-type = ["cdylib", "rlib"]` — `cdylib` produces the `.wasm` component when compiled for `wasm32-wasip2`, and `rlib` lets native integration tests link your router and handlers directly:

```toml
# guest/Cargo.toml
[package]
name = "my-guest"
version = "0.1.0"
edition = "2024"

[lib]
crate-type = ["cdylib", "rlib"]

[dependencies]
axum = { version = "0.8", default-features = false, features = ["json", "query"] }
omnia-sdk = "0.38"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
```

```rust
// guest/src/lib.rs
use axum::extract::Query;
use axum::routing::get;
use axum::{Json, Router};
use omnia_sdk::api::{Client, Context};
use omnia_sdk::{Error, WasiProvider};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub fn router<P: Send + Sync + 'static>(provider: P) -> Router {
    Router::new()
        .route("/", get(hello).post(echo))
        .with_state(Client::new("my-guest", provider))
}

omnia_sdk::http!(router(WasiProvider));

#[derive(Deserialize)]
struct Hello {
    name: Option<String>,
}

#[derive(Serialize)]
struct Reply {
    message: String,
}

async fn hello<P: Send + Sync + 'static>(
    context: Context<P>, Query(input): Query<Hello>,
) -> Result<Json<Reply>, Error> {
    let target = input.name.as_deref().unwrap_or(context.owner());
    Ok(Json(Reply {
        message: format!("hello {target}"),
    }))
}

async fn echo(Json(body): Json<Value>) -> Result<Json<Value>, Error> {
    Ok(Json(json!({ "echo": body })))
}
```

The handlers are plain axum handlers. `Context<P>` is an extractor over the `Client` in the router state: it carries the owner, the provider the handler's capability bounds name, and the request's metadata (`x-request-id` and friends). A handler that needs none of that, like `echo`, takes only axum's own extractors. An `omnia_sdk::Error` answers the JSON `{"error","message"}` envelope at its status.

`omnia_sdk::http!` expands to the `wasi:http/incoming-handler` export (instrumented with an OpenTelemetry span) on `wasm32-wasip2` and to nothing on native targets. Because `omnia-sdk` re-exports `wasip3`, `wit-bindgen`, `omnia-wasi-http`, and `omnia-wasi-otel` internally for the macro expansion, your guest `Cargo.toml` does not need to pin `wasip3` or `wit-bindgen` directly (avoiding version-mismatch deadlocks — see [Troubleshooting](troubleshooting.md#outbound-http-or-spawned-work-inside-a-handler-deadlocks)).

## The host crate

The host is a normal binary; the `runtime!` macro generates its `main`:

```toml
# host/Cargo.toml
[package]
name = "my-host"
version = "0.1.0"
edition = "2024"

[dependencies]
omnia = "0.38"
omnia-wasi-http = "0.38"
omnia-wasi-otel = "0.38"
```

```rust
// host/src/main.rs
use omnia_wasi_http::{HttpDefault, WasiHttp};
use omnia_wasi_otel::{OtelDefault, WasiOtel};

omnia::runtime!({
    hosts: {
        WasiHttp: HttpDefault,
        WasiOtel: OtelDefault,
    }
});
```

`WasiHttp` serves incoming requests and provides outbound HTTP, while `WasiOtel` backs the `http_guest_handle` span emitted by `omnia_sdk::http!`.

That dependency list is complete. `omnia` is the facade over the runtime — the crates behind it (`omnia-core`, `omnia-link`, `omnia-plugin`) are never named in a host's `Cargo.toml` or source, even when the deployment declares a `link:` or `plugin:` block or a hand-written `Backend`; everything the macro emits resolves through `omnia::…`, and it re-exports `anyhow` (`omnia::anyhow::Result`) and `futures` (`omnia::futures::future::BoxFuture`) for the `Backend`/`Wiring` and plugin-store signatures that speak them.

## Build and run

Two steps, always in this order — guest first, then host:

```bash
cargo build -p my-guest --target wasm32-wasip2
cargo run -p my-host -- -v run ./target/wasm32-wasip2/debug/my_guest.wasm
```

(The artifact name uses underscores: `my_guest.wasm`; `-v` shows startup logs.) When `omnia ready` appears, the server is on `localhost:8080`:

```bash
curl http://localhost:8080
curl -X POST http://localhost:8080 -H "Content-Type: application/json" -d '{"hello":"world"}'
```

## Matching versions

Keep every `omnia*` dependency on the same release line (all `0.38`, for example) — the WIT contracts must agree between guest bindings and host implementations. If the line you need is not yet on crates.io, point the dependencies at the git repository instead:

```toml
[patch.crates-io]
omnia = { git = "https://github.com/augentic/omnia.git" }
omnia-sdk = { git = "https://github.com/augentic/omnia.git" }
omnia-wasi-http = { git = "https://github.com/augentic/omnia.git" }
omnia-wasi-otel = { git = "https://github.com/augentic/omnia.git" }
```

Production backends come from the separate [`omnia-backends`](https://github.com/augentic/omnia-backends) repository; its release lines pair with specific omnia lines (see [Production Backends](guides/production-backends.md)).

## Where to go next

- **[Writing Guests](guides/writing-guests.md)** — every guest-side pattern: capabilities, messaging, command mode, tracing.
- **[Composing a Runtime](guides/composing-a-runtime.md)** — modes, backends, and compiled-in deployments for the host.
- **[omnia-exemplar](https://github.com/augentic/omnia-exemplar)** — a complete application-scale reference service built this way.
