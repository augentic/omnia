# omnia-host-macros

Procedural macros for generating host-side WebAssembly Component Runtime infrastructure.

## Overview

This crate provides the `runtime!` macro that generates the necessary runtime infrastructure for executing WebAssembly components with WASI capabilities. Instead of hand-writing the backend bundle, linker wiring, and entry point, you declaratively specify which WASI interfaces and backends your runtime needs.

## Usage

Add `omnia` to your dependencies (the `runtime!` macro is re-exported from the `omnia` crate):

```toml
[dependencies]
omnia = { workspace = true }
```

Then declare your runtime as a map of `Host: Backend` pairs:

```rust,ignore
use omnia_wasi_http::{HttpDefault, WasiHttp};
use omnia_wasi_keyvalue::WasiKeyValue;
use omnia_wasi_otel::{OtelDefault, WasiOtel};
use omnia_redis::Client as Redis;

omnia::runtime!({
    hosts: {
        WasiHttp: HttpDefault,
        WasiOtel: OtelDefault,
        WasiKeyValue: Redis,
    }
});
```

Each key is a **host type** from a `omnia-wasi-*` crate (`WasiHttp`, `WasiKeyValue`, ...); each value is a **backend type** implementing that interface's context trait — an in-tree default (`HttpDefault`, `KeyValueDefault`, ...) or a production client from the [`backends`](https://github.com/augentic/backends) repo.

## Configuration Format

```rust,ignore
omnia::runtime!({
    mode: server,          // optional: `server` (default) or `command`
    config: concat!(env!("CARGO_MANIFEST_DIR"), "/omnia.toml"),  // optional default manifest
    hosts: {
        HostType: BackendType,
        // ...
    }
});
```

- **`mode: server`** — trigger hosts (`WasiHttp`, `WasiMessaging`, `WasiWebSocket`) run servers and drive guests per request.
- **`mode: command`** — the runtime drives the guest's `wasi:cli/run` export once and exits with its status. A backend-less command runtime is valid: `omnia::runtime!({ mode: command });`
- **`config:`** — a path expression compiled into the generated `main` as the default manifest, used only when the command line supplies no positional wasm, `--config`, or `OMNIA_CONFIG`. Anchor it with `env!("CARGO_MANIFEST_DIR")` to make it absolute at compile time.

### Inline manifest keys

Instead of a `config:` path, the deployment can be written inline — the keys mirror the `omnia::Manifest` schema (`omnia.toml` as Rust) and expand to a `Manifest` value compiled in as the same lowest-precedence fallback:

```rust,ignore
omnia::runtime!({
    guests: [
        { id: "responder", source: concat!(env!("CARGO_MANIFEST_DIR"), "/responder.wasm") },
        {
            id: "router",
            source: concat!(env!("CARGO_MANIFEST_DIR"), "/router.wasm"),
            link: ["omnia:link/echo"],   // per-guest host-mediated imports
        },
    ],
    link: ["omnia:shared/log"],          // optional deployment-wide links
    mounts: [
        { name: ".", path: concat!(env!("CARGO_MANIFEST_DIR"), "/workspace"), writable: true },
    ],
    routes: {
        http: [{ prefix: "/", guest: "router" }],
        messaging: [{ topic: "orders.>", guest: "worker" }],
        websocket: [{ route: "chat.*", guest: "ws" }],
    },
    hosts: { /* ... */ }
});
```

Every value is a Rust expression; anchor paths with `env!("CARGO_MANIFEST_DIR")` (relative paths resolve against the run-time working directory). `config:` and the inline keys are mutually exclusive.

## Generated Code

The macro generates a private `runtime` module containing:

### `Backends` bundle

A `Clone` struct with one connected backend per declared `Host: Backend` wiring, plus its `omnia::Backends` impl whose `connect()` connects every backend concurrently. A deployment that declares no backends uses the library's `()` bundle, so nothing is generated.

```rust,ignore
#[derive(Clone)]
struct Backends {
    // ... one field per declared backend
}

impl omnia::Backends for Backends {
    // connect every backend concurrently
    async fn connect() -> Result<Self> { /* ... */ }
}
```

### WASI view accessor impls

For each declared interface, the macro emits the `HasXxx` accessor impl that exposes the bundle's backend to the library's blanket `WasiXxxView for omnia::StoreCtx<Backends>` impl. Most interfaces share one accessor shape; `wasi:http` and `wasi:config` use slightly different ones, handled as special cases in codegen.

### `main` entry point

A `#[tokio::main]` `main` that delegates to `omnia::main::<Backends, Hooks>`, where `Hooks` is a generated `Wiring` impl: `Wiring::link` runs inside `omnia::Runtime::new` to link hosts, connect backends, and assemble the registry; `Wiring::serve` launches each trigger host's `run`. The host runtime is the library `omnia::Runtime<Backends>`; the macro does not emit a runtime type of its own.

The generated `main` handles the `run` subcommand only; to expose `compile`, write a custom `main` that calls `omnia::compile`.

### `run` callable

A blocking `pub fn run(builder: omnia::DeploymentBuilder) -> Result<omnia::ExitStatus>` beside `main`, delegating to `omnia::run::<Backends, Hooks>` with the declared mode applied to the builder. A binary with its own argument surface mounts the runtime in-process through `run` instead of being the generated `main` — it supplies the deployment as an `omnia::Manifest` (loaded with `Manifest::from_config(path)?`, synthesized with `Manifest::from_wasm(path)`, or built fluently with `Manifest::new()`, mounts and links included) via `omnia::DeploymentBuilder::new().manifest(manifest)`, plus argv, and maps the returned `ExitStatus` onto its own exit contract.

Both are re-exported from the generated module as `pub use runtime::{run, main};` (`#[allow(unused_imports)]`, so a nested-module invocation that uses only one stays warning-clean).

## Example: multiple runtime configurations

Different configurations can coexist as modules in one crate:

```rust,ignore
// Minimal HTTP server
mod http_runtime {
    use omnia_wasi_http::{HttpDefault, WasiHttp};

    omnia::runtime!({
        hosts: { WasiHttp: HttpDefault }
    });
}

// Full-featured runtime on NATS
mod full_runtime {
    use omnia_wasi_http::{HttpDefault, WasiHttp};
    use omnia_wasi_keyvalue::WasiKeyValue;
    use omnia_wasi_messaging::WasiMessaging;
    use omnia_wasi_blobstore::WasiBlobstore;
    use omnia_wasi_otel::{OtelDefault, WasiOtel};
    use omnia_nats::Client as Nats;

    omnia::runtime!({
        hosts: {
            WasiHttp: HttpDefault,
            WasiOtel: OtelDefault,
            WasiKeyValue: Nats,
            WasiMessaging: Nats,
            WasiBlobstore: Nats,
        }
    });
}
```

This provides:

- **Better readability**: The configuration is explicit and self-documenting
- **Less boilerplate**: No hand-written bundle, accessor impls, or entry point
- **Type safety**: Backend types are checked against the host's context trait at compile time
- **Flexibility**: Easy to create multiple runtime configurations in the same binary

## License

MIT OR Apache-2.0
