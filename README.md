# Omnia WebAssembly Runtime

[![crates.io](https://img.shields.io/crates/v/omnia.svg)](https://crates.io/crates/omnia)
[![docs.rs](https://docs.rs/omnia/badge.svg)](https://docs.rs/omnia)

A lightweight Rust runtime and SDK for WebAssembly (`wasm32-wasip3`) components, built on [Wasmtime](https://github.com/bytecodealliance/wasmtime).

Omnia can be used for standalone [WASI](https://wasi.dev/) guest development and as the runtime for the [Emery codegen framework](https://github.com/augentic/emery), running agent-generated code in a sandbox that provides controlled access to real infrastructure.

> [!WARNING]
> **Pre-release, pre-hardening software.** While Omnia is usable today, it is still under active development. Expect breaking changes and check [`RELEASES.md`](RELEASES.md) before upgrading.
>
> The runtime has not yet been hardened or independently audited for security. Do not rely on it as the only isolation boundary around hostile code or untrusted input. Layer host and network controls around it, and read the [security model](docs/security-model.md) before running untrusted code.
>
> Report a vulnerability [privately through GitHub](https://github.com/augentic/omnia/security/advisories/new), as the [security policy](https://github.com/augentic/.github/blob/main/SECURITY.md) describes. Until `1.0`, a confirmed vulnerability is tracked as a public `security` issue and fixed on `main`, with no published advisories, CVEs, or backports.

## Quick start

Requires **Rust 1.100+** (`rust-toolchain.toml` installs `wasm32-wasip3` automatically).

```bash
rustup update stable
git clone https://github.com/augentic/omnia && cd omnia
cargo build --example http-wasm --target wasm32-wasip3
cargo run --example http -- -v run ./target/wasm32-wasip3/debug/examples/http_wasm.wasm

# in another terminal, once "omnia ready" is logged
curl -X POST http://localhost:8080 -H "Content-Type: application/json" -d '{"hello": "world"}'
# {"message":"Hello from echo_post!","request":{"hello":"world"}}
```

This runs the [`http`](examples/http) echo example. Its guest writes out by hand the WASI export that the `http::export!` macro below generates.

More runnable examples live in [`examples/`](examples/README.md). To set up your own workspace, see [Starting a New Project](docs/new-project.md).

## Using Omnia

Applications compile to [WebAssembly components](https://component-model.bytecodealliance.org/) using [`omnia-sdk`](crates/sdk) and run inside a native **host** binary assembled with [`omnia::runtime!`](crates/omnia).

Each request runs in a fresh guest instance with bounded memory and execution time. A guest reaches the outside world only through the WASI interfaces its host grants:

| Area | Interfaces |
| ---- | ---------- |
| Communication | [HTTP](crates/wasi-http), [messaging](crates/wasi-messaging), [WebSockets](crates/wasi-websocket) |
| Storage | [key-value](crates/wasi-keyvalue), [blobs](crates/wasi-blobstore), [SQL](crates/wasi-sql), [documents](crates/wasi-docstore) |
| Configuration and credentials | [config](crates/wasi-config), [secrets](crates/wasi-vault), [identity](crates/wasi-identity) |
| Observability | [OpenTelemetry](crates/wasi-otel) |
| AI and code | [model completions](crates/wasi-model), [version control](crates/wasi-vcs) |

Nearly every interface ships with an in-memory default in this repository, so you can build, run, and test locally with no external dependencies. [WASI Interfaces](docs/reference/wasi-interfaces.md) lists each interface's default and production backends.

### What it looks like

A **guest** uses plain [Axum](https://github.com/tokio-rs/axum) handlers (messaging topics and CLI subcommands share the same `Handlers` builder):

```rust
use axum::{Json, routing::post};
use omnia_sdk::{Handlers, Router, http, keyvalue};
use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize)]
struct Item { id: String, name: String }

async fn save(Json(item): Json<Item>) -> omnia_sdk::Result<Json<Item>> {
    keyvalue::default().await?.set_json(&item.id, &item, None).await?;
    Ok(Json(item))
}

fn router() -> Router {
    Handlers::new().route("/items", post(save)).into_router()
}

http::export!(router);
```

A **host** pairs each interface with a backend:

```rust
use omnia_wasi_http::{HttpDefault, WasiHttp};
use omnia_wasi_keyvalue::{KeyValueDefault, WasiKeyValue};
use omnia_wasi_otel::{OtelDefault, WasiOtel};

omnia::runtime!({
    hosts: {
        WasiHttp: HttpDefault,
        WasiKeyValue: KeyValueDefault,
        WasiOtel: OtelDefault,
    }
});
```

Swapping in a production backend is a host-only change: replace `KeyValueDefault` with a client from [`omnia-backends`](https://github.com/augentic/omnia-backends), such as `omnia_redis::Client`, and the `.wasm` guest stays untouched.

## Crates and ecosystem

- [`omnia-sdk`](crates/sdk): guest SDK with WIT bindings, typed helpers, HTTP/messaging/CLI routers, and MCP support.
- [`omnia`](crates/omnia): host composition root for `runtime!`, multi-guest `omnia.toml` manifests, per-guest grants, guest-to-guest linking, plugin loading, and AOT compilation.
- `omnia-wasi-*`: one host crate per interface in the table above, each paired with a backend in `runtime!`.
- [`omnia-test`](crates/test): in-process component test harness over in-memory and scripted backends.
- [`omnia-backends`](https://github.com/augentic/omnia-backends): production backends for PostgreSQL, Redis, Kafka, NATS, MongoDB, Azure, the local filesystem, `git`, `genai`, and `cursor`, plus OS process sandboxing.
- [`omnia-exemplar`](https://github.com/augentic/omnia-exemplar): reference multi-crate guest service and project template.

## Documentation

See the **[Developer Guide](https://augentic.github.io/omnia/)** for full documentation:

- [Getting Started](docs/getting-started.md) and [Starting a New Project](docs/new-project.md)
- [Writing Guests](docs/guides/writing-guests.md), [Composing a Runtime](docs/guides/composing-a-runtime.md), and [Multi-Guest Deployments](docs/guides/multi-guest-deployments.md)
- [Testing Omnia-Based Code](docs/guides/testing-omnia-code.md) and [Troubleshooting](docs/troubleshooting.md)
- [Production Backends](docs/guides/production-backends.md) and [Deploying Omnia](docs/guides/deployment.md)
- [Architecture](docs/Architecture.md), [Security Model](docs/security-model.md), and [WASI Interfaces](docs/reference/wasi-interfaces.md)

## Contributing and license

Bug reports and pull requests are welcome; see [`CONTRIBUTING.md`](CONTRIBUTING.md) and open an issue before starting larger changes.

Licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE).
