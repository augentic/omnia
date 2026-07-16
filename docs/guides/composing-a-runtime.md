# Composing a Runtime

The host runtime is a native binary that loads guests and provides their capabilities. This guide shows how to assemble one with the `runtime!` macro, what the macro generates, and when to drop down to the hand-written alternative.

## The `runtime!` macro

Declare which WASI interfaces your deployment links (`hosts`) and which backend implements each one:

```rust
use omnia_wasi_http::{HttpDefault, WasiHttp};
use omnia_wasi_keyvalue::{KeyValueDefault, WasiKeyValue};
use omnia_wasi_otel::{OtelDefault, WasiOtel};

omnia::runtime!({
    hosts: {
        WasiHttp: HttpDefault,
        WasiOtel: OtelDefault,
        WasiKeyValue: KeyValueDefault,
    }
});
```

Each entry is a `Host: Backend` pair:

- The **host** type (`WasiHttp`, `WasiKeyValue`, ...) is the interface implementation from a `omnia-wasi-*` crate. It links the WASI functions into the wasmtime linker and, for trigger interfaces, runs a server.
- The **backend** type (`HttpDefault`, `KeyValueDefault`, or a production client such as `omnia_redis::Client`) is what the host delegates to. Every backend implements `omnia::Backend` and configures itself from environment variables at startup.

The macro generates:

- a `Backends` bundle holding one connected backend per entry,
- the wiring that links each host and starts each trigger server,
- a `#[tokio::main] main` that parses the CLI (`run` subcommand) and drives the runtime.

The result is a complete binary. Run it with:

```bash
cargo run -- run ./path/to/guest.wasm
```

## Server mode vs command mode

The optional `mode` key selects how the runtime drives guests:

- **`mode: server`** (the default) — the runtime stays up and serves requests. Trigger hosts (`WasiHttp`, `WasiMessaging`, `WasiWebSocket`) listen for traffic and instantiate a fresh guest instance per request.
- **`mode: command`** — the runtime drives the guest's `wasi:cli/run` export exactly once, then exits with the guest's status. Use this for jobs, CLIs, and agent tasks.

```rust
omnia::runtime!({
    mode: command,
    hosts: {
        WasiOtel: OtelDefault,
        WasiModel: ModelDefault,
    }
});
```

In command mode, arguments after `--` on the command line are forwarded to the guest as its argv:

```bash
cargo run --example cli -- run ./target/wasm32-wasip2/debug/examples/cli_wasm.wasm -- greet omnia
```

A backend-less command runtime is valid too: `omnia::runtime!({ mode: command });`.

## Default manifest (`config:`)

The optional `config` key compiles a default manifest path into the generated `main`, used only when the command line supplies no source — no positional wasm, no `--config`, no `OMNIA_CONFIG`:

```rust
omnia::runtime!({
    config: concat!(env!("CARGO_MANIFEST_DIR"), "/deploy/omnia.toml"),
    hosts: {
        WasiHttp: HttpDefault,
    }
});
```

The value is any expression evaluating to a path. Anchoring it with `env!("CARGO_MANIFEST_DIR")` makes it absolute at compile time, so a bare `run` works from any working directory:

```bash
cargo run -- run
```

Explicit sources always win; the compiled-in default is the lowest-precedence fallback.

## Choosing backends

Every WASI interface ships with a default backend that needs no external service, so a development runtime works out of the box. Swapping to production is a one-line change per interface — the guest `.wasm` is untouched:

```rust
// Development
WasiKeyValue: KeyValueDefault,   // in-memory cache

// Production
WasiKeyValue: Redis,             // omnia_redis::Client from the backends repo
```

See [WASI Interfaces](../reference/wasi-interfaces.md) for the full default/production matrix and [Production Backends](production-backends.md) for wiring instructions.

## Backend configuration

Backends read their configuration from environment variables when the runtime starts, via the `FromEnv` trait:

```rust
#[derive(Debug, Clone, FromEnv)]
pub struct ConnectOptions {
    #[env(from = "REDIS_URL", default = "redis://localhost:6379")]
    pub url: String,
}
```

Runtime-wide settings (guest timeout, memory limits, instance pooling) are environment variables as well — see [Configuration](../reference/configuration.md).

## Observability and readiness

- The runtime configures `tracing` and OpenTelemetry at startup. Set `RUST_LOG=info` to see startup logs; set `OTEL_GRPC_URL` to export traces and metrics to an OTLP collector.
- Once bootstrap completes, the runtime logs **`omnia ready`** at `info` level (including the mode and guest count). Orchestrators can watch for this line to detect readiness.

## Hand-written runtimes (advanced)

The macro covers most deployments. If you need a custom entry point — extra CLI flags, non-standard startup order, embedding the runtime in a larger process — implement the `omnia::Wiring` trait yourself and call `omnia::run`, or assemble an `omnia::Runtime<B>` directly from a `DeploymentBuilder`. The [`crates/omnia` README](../../crates/omnia/README.md) lists the public API surface; [Architecture](../Architecture.md) explains how the pieces fit together.

One case that requires this today: the generated `main` handles only the `run` subcommand. To expose ahead-of-time compilation (`compile`, available with the default `jit` feature), call `omnia::compile` from your own `main`.
