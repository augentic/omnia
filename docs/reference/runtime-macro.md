# `runtime!` Macro Reference

Every key the `omnia::runtime!` macro accepts, with exact semantics. The task-oriented walk-through — assembling your first runtime, choosing backends — is [Composing a Runtime](../guides/composing-a-runtime.md); manifests and multi-guest concepts are in [Multi-Guest Deployments](../guides/multi-guest-deployments.md).

## Key summary

| Key | Purpose | You need it when |
| --- | ------- | ---------------- |
| `hosts:` | The `Host: Backend` map — which WASI interfaces are linked and what implements them | Always (except a backend-less command runtime) |
| `mode:` | `server` (default) or `command` | Running jobs/CLIs instead of servers |
| `config:` | Compile in a default manifest *path* | You want `run` with no arguments to work |
| `guests:`, `mounts:`, `link:`, `routes:` | Compile in a default manifest *value* (inline) | Same as `config:`, but self-contained — no TOML file at run time |
| `resolver:` | Resolve-on-miss for unknown guest identities | Guests arrive at run time (multi-tenant, cached artifacts) |
| `command_guest:` | Explicit command-mode routing to one guest identity | More than one static guest, or the command guest comes from the resolver |
| `http_paths:` | Path→identity hook for unrouted HTTP requests | The deployment owns HTTP routing (e.g. per-tenant paths) |
| `http_listener:` | Adopt a pre-bound TCP listener | The embedding process controls the socket (port 0 tests, socket activation) |
| `program:` | Raw argv passthrough — removes the host CLI | Shipping a binary whose CLI belongs entirely to the guest |

## `hosts:`

Each entry is a `Host: Backend` pair:

- The **host** type (`WasiHttp`, `WasiKeyValue`, ...) is the interface implementation from an `omnia-wasi-*` crate. It links the WASI functions into the wasmtime linker and, for trigger interfaces, runs a server.
- The **backend** type (`HttpDefault`, `KeyValueDefault`, or a production client such as `omnia_redis::Client`) is what the host delegates to. Every backend implements `omnia::Backend` and configures itself from environment variables at startup.

The macro generates a `Backends` bundle (one connected backend per entry), the wiring that links each host and starts each trigger server, and a `#[tokio::main] main` that parses the CLI (`run` subcommand) and drives the runtime.

## `mode:`

- **`mode: server`** (default) — the runtime stays up and serves requests. Trigger hosts (`WasiHttp`, `WasiMessaging`, `WasiWebSocket`) listen for traffic and instantiate a fresh guest instance per request.
- **`mode: command`** — the runtime drives the guest's `wasi:cli/run` export exactly once, then exits with the guest's status. Unlike server triggers, command mode applies no `GUEST_TIMEOUT_MS` wall-clock cap — to the run itself or to any link dispatch made along its call chain.

In command mode, arguments after `--` on the command line are forwarded to the guest as its argv (`args[0]` is the program name, supplied by the runtime). This `run … -- …` grammar applies to every generated binary *except* one built with [`program:`](#program-raw-argv-passthrough), which disables the host CLI entirely.

A backend-less command runtime is valid: `omnia::runtime!({ mode: command });`.

By default, command mode routes to the sole static guest exporting `wasi:cli/run`; a deployment with no exporter is inert and exits `0`. Use [`command_guest:`](#command_guest-explicit-command-routing) to route explicitly.

## `config:` (default manifest path)

Compiles a default manifest path into the generated `main`, used only when the command line supplies no source — no positional wasm, no `--config`, no `OMNIA_CONFIG`:

```rust
omnia::runtime!({
    config: concat!(env!("CARGO_MANIFEST_DIR"), "/deploy/omnia.toml"),
    hosts: {
        WasiHttp: HttpDefault,
    }
});
```

The value is any expression evaluating to a path. Anchoring it with `env!("CARGO_MANIFEST_DIR")` makes it absolute at compile time, so a bare `run` works from any working directory. Explicit sources always win; the compiled-in default is the lowest-precedence fallback.

`config:` and the inline manifest keys are mutually exclusive — a runtime compiles in a manifest path or a manifest value, not both.

## Inline manifest keys (`guests:`, `mounts:`, `link:`, `routes:`)

Everything `omnia.toml` expresses can also be written directly in the macro, mirroring the `omnia::Manifest` schema. The macro expands the keys to a `Manifest` value compiled into the generated `main` as the same lowest-precedence fallback as `config:`:

```rust
omnia::runtime!({
    guests: [
        {
            id: "responder",
            source: concat!(env!("CARGO_MANIFEST_DIR"), "/guests/responder.wasm"),
        },
        {
            id: "router",
            source: concat!(env!("CARGO_MANIFEST_DIR"), "/guests/router.wasm"),
            link: ["omnia:link/echo"],       // per-guest host-mediated imports
        },
    ],
    link: ["omnia:shared/log"],              // optional deployment-wide links
    mounts: [
        { name: ".", path: concat!(env!("CARGO_MANIFEST_DIR"), "/workspace"), writable: true },
    ],
    routes: {
        http: [{ prefix: "/", guest: "router" }],
        messaging: [{ topic: "orders.>", guest: "worker" }],
        websocket: [{ route: "chat.*", guest: "ws" }],
    },
    hosts: {
        WasiHttp: HttpDefault,
    }
});
```

- Each value is any Rust expression evaluating to the field's type (strings for ids, interfaces, and route keys; paths or embedded bytes for `source`, paths for mount `path`; a bool for `writable`, which defaults to `false`).
- Relative paths resolve against the process working directory at run time, so anchor them with `env!("CARGO_MANIFEST_DIR")` as with `config:`.

### Embedding a guest (`source:` bytes)

`source:` also accepts component bytes, embedding the guest in the host binary — no adjacent `.wasm` file at run time:

```rust
omnia::runtime!({
    guests: [{
        id: "app",
        source: include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../target/wasm32-wasip2/debug/examples/app_wasm.wasm",
        )),
    }],
    hosts: { /* ... */ }
});
```

Two things to know:

- **The guest artifact must exist when the host crate compiles** — `include_bytes!` reads it at build time, so the two-step build order (guest first, then host) becomes a hard requirement rather than a run-time one. This is why the repository's own examples stay path-based.
- **Embed raw `.wasm`, not `omnia compile` output.** Raw wasm is safe and JIT-compiles at startup (the `jit` feature is on by default). Embedded pre-compiled bytes are native code and are rejected by the safe build, same as pre-compiled paths; they require the programmatic `DeploymentBuilder::precompiled()` unsafe build — see the [security model](../security-model.md). Single-binary shipping of a *pre-compiled* guest is the [embedded-guest design](../../rfcs/embedded-guest.md).

The [`guest-link`](../../examples/guest-link/runtime.rs) example is built this way; its [`omnia.toml`](../../examples/guest-link/Omnia.toml) expresses the same deployment as a file for `--config`.

## `resolver:` (resolve-on-miss)

**You need this when guests are not all known at compile time** — for example multi-tenant deployments where a tenant's guest is fetched from a cache or registry the first time it is addressed.

The value is any expression evaluating to a type implementing `omnia::GuestResolver`; it is consulted on dispatch-path registry misses (see [dynamic guest registration](../../rfcs/guest-resolution.md)). A resolver implies a *dynamic* deployment: the guest set may start empty (an invocation with a resolver and no `guests:` is the fully dynamic deployment), and with one or more static guests the mark is a no-op. The compiled-in resolver is part of the *binary*, not the manifest — a TOML supplied via `--config` still runs with it, and resolution policy (id grammar, artifact layout, verification) stays code the deployment owns.

## `command_guest:` (explicit command routing)

**You need this when command mode should not rely on the sole-exporter default** — with several static guests, or a command guest that arrives through the resolver.

The value (any expression evaluating to a guest identity) routes command mode explicitly instead of the sole-static-exporter catch-all. It requires `mode: command`. With several static guests — or a resolver-supplied command guest — explicit routing is the safer form: a future guest accidentally exporting `wasi:cli/run` cannot flip the routing.

`command_guest` sends the identity through the ordinary registry lookup — and hence resolve-on-miss when a `GuestResolver` is installed — so a fully dynamic deployment may start empty and fault its command guest in on the first run. This leg is fail-closed: an identity nothing supplies, a resolver failure, or a resolved component that does not export `wasi:cli/run` fails the run instead of exiting inert.

The same routing is available programmatically:

```rust
let builder = omnia::DeploymentBuilder::new()
    .dynamic()
    .command_guest("app@1.2.0")
    .program_name("app")
    .resolver(resolver)
    .args(argv);
host::run(builder).await
```

`program_name` overrides the deployment name used for telemetry and prepended to guest argv as `argv[0]` (the default remains the manifest name).

## `http_paths:` (path routing hook)

**You need this when the deployment owns HTTP routing** — mapping request paths to guest identities in code (for example `/tenants/<id>/...` schemes), instead of static `[[route.http]]` prefixes.

The value is any expression evaluating to `Fn(&str) -> Option<omnia::GuestId>` (`+ Send + Sync + 'static`); it maps a request path no static `[[route.http]]` prefix matches to a guest identity, which then goes through the ordinary registry lookup — and hence resolve-on-miss when a `resolver:` is installed.

Installing the hook makes the deployment own HTTP routing outright: the capability default is off (a sole `wasi:http` exporter never becomes a catch-all). Outcomes:

- The hook declines a path, **or** claims an identity nothing supplies (the resolver's definitive miss, e.g. an unknown tenant) → an ordinary 404.
- A genuine fault on a claimed path — resolution failed, or the routed guest lacks a `wasi:http` handler export → an error-logged 500, never hidden as a miss.
- Without the hook, an unmatched path stays a routine debug-level 404 — a routes-only server does not claim every path.

Deployments assembled programmatically supply the same hook through `DeploymentBuilder::http_paths`.

## `http_listener:` (pre-bound listener)

**You need this when the process, not the runtime, must own the socket** — binding port 0 in tests, inheriting a socket from a supervisor, or coordinating with another server in the same process.

The value is any expression evaluating to `anyhow::Result<std::net::TcpListener>`, evaluated at the top of the generated `main`: writing the key means supplying a listener, and an `Err` is a startup failure. The HTTP trigger adopts the pre-bound listener (it serves on that exact address instead of binding `HTTP_ADDR` itself), and every guest store sees `HTTP_ADDR` set to the listener's local address — injected with override semantics, so it wins over any inherited value.

A supplied listener that no `wasi:http`-capable guest (and no `http_paths:` hook) could ever serve fails startup rather than silently dropping the socket. Deployments assembled programmatically supply the listener through `DeploymentBuilder::http_listener`.

## `program:` (raw argv passthrough)

**You need this when you ship a binary whose command line belongs entirely to the guest** — a product CLI where `mybin greet Ada` must work, not `mybin run guest.wasm -- greet Ada`.

`program:` disables the host `run` grammar entirely: the binary's argv belongs to the guest. There is no `run` subcommand and no `--config`/`OMNIA_CONFIG`/positional-wasm override — the deployment compiled into the binary (or supplied by the resolver) is the only source, by design. The key's value (any expression evaluating to a string) becomes the program name used for telemetry and prepended to guest argv as `argv[0]`. This is pure opt-in: a binary without `program:` keeps the `run` grammar byte-for-byte.

Two host log flags are reserved on this path: `--debug` and `--quiet`, anywhere in argv, are peeled before the guest sees them and select the host log preset (see [Host log flags](configuration.md#host-log-flags-program-binaries)). Everything else passes through untouched.

`program:` requires `mode: command` and either a compiled-in manifest (`config:` or inline keys) or `resolver:` plus `command_guest:` (the fully dynamic shape) — anything else is a compile-time error, since a direct command with nothing to run could never work.

## Composing the keys

The [`command-resolver`](../../examples/command-resolver/runtime.rs) example composes `program:`, `resolver:`, and `command_guest:` into a complete resolver-backed command deployment — static guests plus resolve-on-miss for everything else — with no handwritten `main`:

```rust
omnia::runtime!({
    mode: command,
    program: "specify-example",
    guests: [
        { id: "specify", source: engine_component_path() },
        { id: "target:mock", source: mock_target_path() },
    ],
    mounts: [
        { name: "project", path: project_root(), writable: true },
        { name: "store", path: store_root(), writable: true },
    ],
    resolver: CacheResolver::new(),
    command_guest: "specify",
    hosts: {
        WasiHttp: HttpDefault,
        WasiOtel: OtelDefault,
        WasiModel: Cursor,
    }
});
```
