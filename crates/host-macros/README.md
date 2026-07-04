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
    hosts: {
        HostType: BackendType,
        // ...
    }
});
```

- **`mode: server`** — trigger hosts (`WasiHttp`, `WasiMessaging`, `WasiWebSocket`) run servers and drive guests per request.
- **`mode: command`** — the runtime drives the guest's `wasi:cli/run` export once and exits with its status. A backend-less command runtime is valid: `omnia::runtime!({ mode: command });`

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
