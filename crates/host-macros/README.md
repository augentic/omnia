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

Each key is a **host type** from a `omnia-wasi-*` crate (`WasiHttp`, `WasiKeyValue`, ...); each value is a **backend type** implementing that interface's context trait — an in-tree default (`HttpDefault`, `KeyValueDefault`, ...) or a production client from the [`omnia-backends`](https://github.com/augentic/omnia-backends) repo.

A backend may carry a connect-options expression — `WasiBlobstore: Filesystem(opts)` lowers to `Backend::connect_with(opts)` instead of the env-sourced `Backend::connect()`, compiling the configuration in. Rows sharing a backend type share one connection, so their options must be written identically on every row (or omitted on every row); empty parentheses are rejected.

## Configuration Format

```rust,ignore
omnia::runtime!({
    mode: server,          // optional: `server` (default) or `command`
    manifest: concat!(env!("CARGO_MANIFEST_DIR"), "/omnia.toml"),  // optional default manifest
    hosts: {
        HostType: BackendType,
        // ...
    }
});
```

- **`mode: server`** — trigger hosts (`WasiHttp`, `WasiMessaging`, `WasiWebSocket`) run servers and drive guests per request.
- **`mode: command`** — the runtime drives the guest's `wasi:cli/run` export once and exits with its status. A backend-less command runtime is valid: `omnia::runtime!({ mode: command });` With a compiled-in deployment (`manifest:` or inline manifest keys), the binary is a *direct command*: no host `run` grammar, argv passes to the guest verbatim. Command mode routes to the sole static `wasi:cli/run` exporter, or to the guest entry marked `command: true`.
- **`manifest:`** — a path expression compiled into the generated `main` as the default manifest, used only when the command line supplies no positional wasm, `--manifest`, or `OMNIA_MANIFEST`. Anchor it with `env!("CARGO_MANIFEST_DIR")` to make it absolute at compile time.

### Inline manifest keys

Instead of a `manifest:` path, the deployment can be written inline — the keys mirror the `omnia::Manifest` schema (`omnia.toml` as Rust) and expand to a `Manifest` value compiled in as the same lowest-precedence fallback:

```rust,ignore
omnia::runtime!({
    guests: [                                        // the components embedded in the binary
        {
            name: "responder",                       // optional: the path's file stem otherwise
            path: concat!(env!("CARGO_MANIFEST_DIR"), "/responder.wasm"),
            routes: { messaging: ["orders.>"] },     // inbound routes targeting this guest
        },
        {
            path: concat!(env!("CARGO_MANIFEST_DIR"), "/router.wasm"),   // the guest `router`
            routes: { http: ["/"], websocket: ["chat.*"] },
        },
    ],
    registries: include_str!("wasm-pkg.toml"),       // optional: where `package:` guests are fetched from
    mounts: [
        { name: ".", path: concat!(env!("CARGO_MANIFEST_DIR"), "/workspace"), writable: true },
    ],
    hosts: { /* ... */ }
});
```

`guests:` lists the components compiled into the deployment. Each entry's `path:` is embedded with `include_bytes!`, so it is a string literal or a macro that expands to one — `concat!(env!("CARGO_MANIFEST_DIR"), ..)` anchors a path in the crate, `env!("GUEST_WASM")` reads one a `build.rs` emitted — and the artifact — raw `.wasm` or `omnia compile` output — must exist when the host crate compiles. The guest is named by the file's stem (`responder.wasm` is `responder`) unless `name:` says otherwise; the name is the identity it registers under and is dispatched by, so a caller dispatching to `responder` finds it whatever the file was called. `routes:` declares the guest's inbound routes, one pattern list per trigger, with the declaring guest as the implicit target; `command: true` marks the command-mode target. `package: "ns:name@1.0.0"` in place of `path:` names an exact registry package, fetched at the guest's first use — the first time a route, the command drive, a link call, a host dispatch, or an `omnia:plugins/loader.load` names it — and named by the reference without its version (`acme:tool@1.2.3` is `acme:tool`) unless `name:` says otherwise; `digest: "sha256:…"` pins the bytes either kind must hash to. A package admits raw `.wasm` alone; embedded bytes admit either format. `registries:` is the wasm-pkg configuration (the file's contents, so `include_str!` is the idiom) `package:` guests are fetched through; absent, their loads are refused. `mounts:` are the directories preopened into every guest; a read-only one is also a root the loader reads a component from when a guest names a path beneath it, a writable one never is. Every other value is a Rust expression; anchor mount paths with `env!("CARGO_MANIFEST_DIR")` (relative paths resolve against the run-time working directory). `manifest:` and the inline keys are mutually exclusive — a manifest-file deployment declares the same things in the TOML (`[[guest]]`, `[registries] path = "wasm-pkg.toml"`, `[[mount]]`), where a guest's component is read at its first use rather than embedded.

Nothing declares what guests call between themselves: every import a guest makes outside the runtime's own `wasi:` and `omnia:` namespaces is relayed to the guest exporting it (the `link` feature), and nothing in the invocation opts into the guest loader — omnia links its host at assembly whenever it is built with the `loader` feature (worlds that do not import `omnia:plugins/loader` never see it), and a `load` of a declared name goes through the same first-use seam every other use does.

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

### Bundle accessor impls

For each `hosts:` row, the macro emits one uniform `omnia::Provides<Ctx>` impl exposing the bundle's backend field; the generic `omnia::StoreView` blanket on `omnia::StoreCtx` turns it into the host's linker-facing view. The impl is keyed by the host type itself — its `omnia::HostCtx` impl carries the borrow shape, so no names are derived from the host and third-party hosts wire the same way. The one exception is `wasi:http`, whose linker-facing view trait is foreign (`wasmtime-wasi-http`): its row is keyed to the core-owned `omnia::HttpCtx` carrier instead.

### `main` entry point

A `#[tokio::main]` `main` that delegates to `omnia::main::<Backends, Hooks>` with the invoking crate's package name as the program name (telemetry's component name and, in command mode, the guest's `argv[0]`), where `Hooks` is a generated `pub struct` implementing `omnia::Wiring<B>` for every bundle `B` that `Provides` each declared host's context (the generated `Backends` among them): `Wiring::link` runs inside `omnia::Runtime::new` to link hosts before backends connect and the registry assembles; `Wiring::serve` launches each trigger host's `run`. The guest loader is not wired by the macro: `omnia::Deployment::assemble` links its host and installs the deployment's policy for every runtime alike. The host runtime is the library `omnia::Runtime<Backends>`; the macro does not emit a runtime type of its own.

The generated `main` handles the `run` subcommand only; to expose `compile`, write a custom `main` that calls `omnia::compile`.

### `run` callable

A blocking `pub fn run(builder: omnia::DeploymentBuilder) -> Result<omnia::ExitStatus>` beside `main`. It applies the declared mode, builds the deployment, and delegates to `omnia::run::<Backends, Hooks>`. A binary with its own argument surface mounts the runtime in-process through `run` instead of being the generated `main` — it supplies the deployment as an `omnia::Manifest` (loaded with `Manifest::load(path)?`, synthesized with `Manifest::from_wasm(path)?`, or built fluently with `Manifest::new()`, mounts and registries included) via `omnia::DeploymentBuilder::new().manifest(manifest)`, plus argv, and maps the returned `ExitStatus` onto its own exit contract.

### `manifest` and `run_with`

`pub fn manifest() -> omnia::ManifestSource` returns the compiled-in deployment (`manifest:` path or inline manifest keys; an empty inline manifest when neither is declared), and `pub async fn run_with<B>(builder, backends: B) -> Result<omnia::ExitStatus>` builds the builder and drives the resulting deployment through `Hooks` over a bundle already in hand, connecting nothing. Together with `Hooks` they let a test run the binary's own wiring over test backends: `omnia_test::host::Deployment::from(runtime::manifest())` overlays the compiled-in manifest, and `run_with::<runtime::Hooks, _>` drives it.

All five are re-exported from the generated module as `pub use runtime::{Hooks, main, manifest, run, run_with};` (`#[allow(unused_imports)]`, so a nested-module invocation that uses only some stays warning-clean).

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
