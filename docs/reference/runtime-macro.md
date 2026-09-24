# `runtime!` Macro Reference

Every key the `omnia::runtime!` macro accepts, with exact semantics. The task-oriented walk-through — assembling your first runtime, choosing backends — is [Composing a Runtime](../guides/composing-a-runtime.md); manifests and multi-guest concepts are in [Multi-Guest Deployments](../guides/multi-guest-deployments.md).

## Key summary

| Key | Purpose | You need it when |
| --- | ------- | ---------------- |
| `hosts:` | The `Host: Backend` map — which WASI interfaces are linked and what implements them | Always (except a backend-less command runtime) |
| `mode:` | `server` (default) or `command` | Running jobs/CLIs instead of servers |
| `manifest:` | Compile in a default manifest *path* | You want `run` with no arguments to work |
| `guests:`, `registries:`, `mounts:` | Compile in a default manifest *value* (inline), the guests embedded | Same as `manifest:`, but self-contained — no TOML file and no `.wasm` beside the binary at run time |

There is no key for raw argv passthrough: a command-mode runtime with a compiled-in deployment is a [direct command](#direct-commands-raw-argv-passthrough) automatically.

## `hosts:`

Each entry is a `Host: Backend` pair:

- The **host** type (`WasiHttp`, `WasiKeyValue`, ...) is the interface implementation from an `omnia-wasi-*` crate. It links the WASI functions into the wasmtime linker and, for trigger interfaces, runs a server.
- The **backend** type (`HttpDefault`, `KeyValueDefault`, or a production client such as `omnia_redis::Client`) is what the host delegates to. Every backend implements `omnia::Backend` and configures itself from environment variables at startup.

The macro generates a `Backends` bundle (one connected backend per entry, with one `omnia::Provides` accessor impl per row), the wiring that links each host and runs every host's `Server::run` (a no-op for capability hosts; the serve loop for trigger hosts), and a `#[tokio::main] main` that parses the CLI (`run` subcommand) and drives the runtime.

### Connect options

A backend name may carry a connect-options expression: `Host: Backend(options)` lowers to `Backend::connect_with(options)` instead of the env-sourced `Backend::connect()`, compiling configuration into the binary — a fixed storage root, a scripted test backend carrying state — rather than reading it from the environment at startup:

```rust
omnia::runtime!({
    hosts: {
        WasiKeyValue: Filesystem(FilesystemOptions::at(".omnia/storage")),
        WasiBlobstore: Filesystem(FilesystemOptions::at(".omnia/storage")),
        WasiOtel: OtelDefault,
    }
});
```

The expression's type is the backend's `Backend::ConnectOptions`. Rows sharing a backend type share one connection, so their options must be written identically on every row (or omitted on every row) — a mismatch is a compile error, as are empty parentheses (drop the `()` to connect from the environment).

## `mode:`

- **`mode: server`** (default) — the runtime stays up and serves requests. Trigger hosts (`WasiHttp`, `WasiMessaging`, `WasiWebSocket`) listen for traffic and instantiate a fresh guest instance per request.
- **`mode: command`** — the runtime drives the guest's `wasi:cli/run` export exactly once, then exits with the guest's status. Unlike server triggers, command mode applies no `GUEST_TIMEOUT_MS` wall-clock cap — to the run itself or to any link dispatch made along its call chain.

Command mode has two entry surfaces, chosen by whether the deployment is compiled in. With a compiled-in deployment (`manifest:` or inline manifest keys) the binary is a [direct command](#direct-commands-raw-argv-passthrough): no host CLI, argv passes to the guest verbatim. Without one, the standard `run … -- …` grammar applies — arguments after `--` are forwarded to the guest as its argv (`args[0]` is the program name, supplied by the runtime).

A backend-less command runtime is valid: `omnia::runtime!({ mode: command });`.

By default, command mode routes to the sole static guest exporting `wasi:cli/run`; a deployment with no exporter is inert and exits `0`. With several exporters, mark one guest entry `command: true` — see [Command routing](#command-routing-command-true).

## `manifest:` (default manifest path)

Compiles a default manifest path into the generated `main`, used only when the command line supplies no source — no positional wasm, no `--manifest`, no `OMNIA_MANIFEST`:

```rust
omnia::runtime!({
    manifest: concat!(env!("CARGO_MANIFEST_DIR"), "/deploy/omnia.toml"),
    hosts: {
        WasiHttp: HttpDefault,
    }
});
```

The value is any expression evaluating to a path. Anchoring it with `env!("CARGO_MANIFEST_DIR")` makes it absolute at compile time, so a bare `run` works from any working directory. Explicit sources always win; the compiled-in default is the lowest-precedence fallback.

`manifest:` and the inline manifest keys are mutually exclusive — a runtime compiles in a manifest path or a manifest value, not both.

## Inline manifest keys (`guests:`, `registries:`, `mounts:`)

The deployment `omnia.toml` expresses can also be written directly in the macro, mirroring the `omnia::Manifest` schema. The macro expands the keys to a `Manifest` value compiled into the generated `main` as the same lowest-precedence fallback as `manifest:` — with one difference: an inline guest is *embedded*, its component bytes compiled into the binary, where a manifest file's guest is read from disk at start.

```rust
omnia::runtime!({
    guests: [                                        // the components embedded in the binary
        {
            name: "responder",                       // optional: the file's stem otherwise
            path: concat!(env!("CARGO_MANIFEST_DIR"), "/guests/responder.wasm"),
            routes: {
                messaging: ["orders.>"],             // inbound routes targeting this guest
            },
        },
        {
            path: concat!(env!("CARGO_MANIFEST_DIR"), "/guests/router.wasm"),   // the guest `router`
            routes: {
                http: ["/"],
                websocket: ["chat.*"],
            },
        },
    ],
    registries: include_str!("wasm-pkg.toml"),       // optional: where on-demand `package:` guests are fetched from
    mounts: [
        { name: ".", path: concat!(env!("CARGO_MANIFEST_DIR"), "/workspace"), writable: true },
    ],
    hosts: {
        WasiHttp: HttpDefault,
    }
});
```

- `guests:` is a list of entries `{ path | package, name?, routes?, command?, on_demand?, digest? }` — see [Guest entries](#guest-entries-pathpackage-name-routes-command-on_demand-digest).
- `registries:` is the wasm-pkg configuration an on-demand `package:` guest is fetched through — see [`registries:`](#registries).
- `mounts:` are the directories preopened into every guest sandbox; each entry is `{ name, path, writable? }` (`writable` a literal bool, `false` by default). A mount is a data grant only — the guest loader never reads code through one. A relative mount path resolves against the process working directory at run time, so anchor it with `env!("CARGO_MANIFEST_DIR")` as with `manifest:`.
- Every value other than a guest's `path:` is any Rust expression evaluating to the field's type (strings for names and route patterns, paths for mounts, a `&'static str` or `String` of TOML for `registries`).
- `manifest:` and the inline keys are mutually exclusive; a manifest file declares the same things as `[[guest]]`, `[registries]`, and `[[mount]]` — see [Configuration](configuration.md).

Nothing declares what guests call between themselves. Every import a guest makes outside the runtime's own namespaces (`wasi:`, `omnia:`) is relayed to the guest exporting it, whichever it is, and an import no guest exports fails at boot — see [Multi-Guest Deployments](../guides/multi-guest-deployments.md#guests-calling-guests).

### Guest entries (`path`/`package`, `name`, `routes`, `command`, `on_demand`, `digest`)

- **`path: <literal or macro>`** — the component, embedded with `include_bytes!`. Because `include_bytes!` takes a string literal (or a macro expanding to one), so does `path:`: `"guest.wasm"` reads relative to the invoking source file, `concat!(env!("CARGO_MANIFEST_DIR"), "/guests/app.wasm")` anchors a path in the crate, and `env!("APP_WASM")` reads a path a `build.rs` emitted with `cargo:rustc-env`. A computed path (`engine_component_path()`) is refused at compile time; a component read at start is a `manifest:` file's `[[guest]]`. The artifact must exist when the host crate compiles; it may be raw `.wasm`, compiled at startup, or `omnia compile` output, deserialized at startup (see the [security model](../security-model.md)). The repository's `examples` package compiles the guests its `cli-static` and `guest-link` hosts embed in a `build.rs` (through `omnia_test::build::Components`), so `cargo run --example cli-static` needs no guest build first.
- **`package: <expr>`** — instead of `path:`, an exact `namespace:name@version` package reference the guest loader fetches on first load through [`registries:`](#registries). A package guest has no file stem, so it requires `name:`, and it is fetched on first load, so it requires `on_demand: true`; `path:` and `package:` together are a compile error.
- **`name: <expr>`** — the guest's name: the `GuestId` it registers under, is dispatched to, and is loaded as by a guest's `loader.load("name")`. Omitted, the `path:` file's stem names it — `guests/router.wasm` is `router`, `cli_wasm.wasm` is `cli_wasm`. Give a name when the file is not called what callers dispatch to, or when two files share a stem; two guests of one name are refused at startup.
- **`routes: { http: [..], messaging: [..], websocket: [..] }`** — the guest's inbound routes, one pattern list per trigger (`http` prefixes, `messaging` topics, `websocket` routes), with the declaring guest as the implicit target. There is no top-level `routes:` key.
- **`command: true`** (a literal bool) — marks the guest as the command-mode target — see [Command routing](#command-routing-command-true).
- **`on_demand: true`** (a literal bool) — the guest is admitted when a guest first `loader.load`s it by name rather than at boot. An on-demand guest is still part of the compiled-in allow-list — the loader admits no name the invocation does not declare — but it takes no `routes:` and cannot be the `command:` guest, since trigger routing is built at boot; either combination is a compile error.
- **`digest: "sha256:<64 hex>"`** (a string literal) — the digest the guest's bytes must hash to, checked before wasmtime sees them, at boot or on demand alike; a malformed pin is a compile error. Pin every on-demand guest whose bytes could change between boot and first load.

Any other guest key is a compile error naming the seven.

### `registries:`

The [wasm-pkg client configuration](https://github.com/bytecodealliance/wasm-pkg-tools) (TOML: `default_registry`, `namespace_registries`, `package_registry_overrides`, per-registry `[registry."..."]` backend settings) that an on-demand `package:` guest is fetched through when a guest first loads it. The expression is the configuration's *contents* — `include_str!("wasm-pkg.toml")` is the idiom — lowered to `omnia::RegistryConfig::contents(..)` on the compiled-in manifest; a manifest file names a path instead (`[registries] path = "wasm-pkg.toml"`). A package resolves to its package override, else its namespace's registry, else `default_registry`; a package the configuration routes nowhere is refused, naming its namespace, before any registry is dialled. The contents are parsed when the runtime assembles, so a malformed configuration fails startup, not the first load. Without `registries:`, every package guest's load is refused typed. Nothing a loading guest passes names a registry: the invocation chooses both the package and where it is fetched from.

Nothing in the invocation opts into the guest loader. When omnia is built with its non-default `loader` feature (`omnia = { version = "...", features = ["loader"] }`), `Deployment::assemble` links the `omnia:plugins/loader` host beside WASI — worlds that do not import it never see it — and installs the invocation's `on_demand` guests as the loader's table; without the feature a manifest declaring `registries` is refused at startup, and a guest importing the loader fails at instantiation. An embedder wanting a different registry — a caching `RegistryClient::cached`, a source of its own — selects it on the built deployment with `Deployment::registry_source(..)` before `assemble`. The `loader` and `link` features are independent: loading a guest and dispatching to it are two capabilities, and a deployment that does both enables both.

Because mounts, on-demand guests, and registries are deployment data, a test can overlay them without touching the binary's declaration: `omnia_test::host::Deployment::from(runtime::manifest()).mount(scratch.mount(false))` replaces the binary's `.` mount (mounts dedup by name, last wins, before any directory opens) and `runtime::run_with(...)` drives the same generated wiring over in-memory backends — see [Testing omnia code](../guides/testing-omnia-code.md).

## Command routing (`command: true`)

**You need this when command mode should not rely on the sole-exporter default** — a deployment with several static guests exporting `wasi:cli/run`.

Marking one guest entry `command: true` routes command mode to that guest instead of the sole-static-exporter catch-all. The mark requires `mode: command`, and at most one guest may carry it — several are rejected at compile time (and, for a TOML manifest, at manifest validation). With several exporters the mark is also the safer form: a future guest accidentally exporting `wasi:cli/run` cannot flip the routing.

```rust
omnia::runtime!({
    mode: command,
    guests: [
        { path: "app.wasm", command: true },
        { path: "helper.wasm" },
    ],
});
```

The same mark is available in `omnia.toml` (`command = true` on a `[[guest]]` entry — see [Configuration](configuration.md)) and programmatically (`omnia::GuestEntry::new(name, source).command()`). This leg is fail-closed: a marked identity nothing supplies, or one whose component does not export `wasi:cli/run`, fails the run instead of exiting inert.

The program name — telemetry's component name and, in command mode, the `argv[0]` prepended to the guest's arguments — is the invoking crate's package name (`CARGO_PKG_NAME`), which the macro compiles in; `DeploymentBuilder::program_name` sets it on a programmatic deployment, and one that sets none is `omnia`.

## Direct commands (raw argv passthrough)

**Shipping a binary whose command line belongs entirely to the guest** — a product CLI where `mybin greet Ada` must work, not `mybin run guest.wasm -- greet Ada` — needs no key at all: a `mode: command` runtime with a compiled-in deployment (`manifest:` or inline manifest keys) is a *direct command*.

A direct command has no host `run` grammar: the binary's argv belongs to the guest. There is no `run` subcommand and no `--manifest`/`OMNIA_MANIFEST`/positional-wasm override — the deployment compiled into the binary is the only source, by design. The program name used for telemetry and prepended to guest argv as `argv[0]` is the crate's package name.

Every argument passes through to the guest untouched. The host also *reads* the verbosity flags on their way past — `-v`/`--verbose` and `-q`/`--quiet`, repeated as `-vv`, before any `--` — to set the process tracing level for the host console and every guest's `RUST_LOG` before the guest runs (see [Verbosity flags](configuration.md#verbosity-flags)); it removes nothing, so the guest declares the same flags in its own grammar by flattening `omnia_sdk::api::command::Verbosity`, which lists them in its help and completions, accepts them, and refuses `-v` beside `-q` as its own usage error. Every other flag is the guest's to define; a guest that wants one of its own to set its tracing defaults reloads with `omnia_wasi_otel::set_filter` once it has parsed argv, and its `RUST_LOG` still refines whatever it sets.

A `mode: command` runtime *without* a compiled-in deployment keeps the `run` grammar byte-for-byte — with no other way to name the guest, the positional wasm path and `--manifest` remain the entry surface.

## Composing the keys

The [`cli-static`](../../examples/cli-static/runtime.rs) example composes the inline manifest keys into a complete direct command deployment with no handwritten `main`; a product runtime that also loads adapters at run time composes every key:

```rust
omnia::runtime!({
    mode: command,
    guests: [
        { path: concat!(env!("OUT_DIR"), "/engine.wasm"), command: true },   // the guest `engine`
        { name: "target:mock", path: concat!(env!("CARGO_MANIFEST_DIR"), "/mocks/target.wasm") },
    ],
    registries: include_str!("wasm-pkg.toml"),
    mounts: [
        { name: "project", path: project_root(), writable: true },
        { name: "store", path: store_root(), writable: true },
    ],
    hosts: {
        WasiHttp: HttpDefault,
        WasiOtel: OtelDefault,
        WasiModel: Cursor,
    }
});
```

## Generated items

The invocation expands to a private `runtime` module and re-exports five items from it:

| Item | Shape | Use |
| ---- | ----- | --- |
| `main` | `#[tokio::main] fn main() -> ExitCode` | The binary's entry point: connects the declared backends from the environment and drives the compiled-in deployment. |
| `run` | blocking `fn run(DeploymentBuilder) -> Result<ExitStatus>` | Build the builder, then mount the runtime in-process from a binary with its own argument surface. |
| `Hooks` | `pub struct Hooks` implementing `omnia::Wiring<B>` | The generated wiring — `link`, `serve` — generic over any bundle `B` that `Provides` each declared host's context. |
| `manifest` | `fn manifest() -> ManifestSource` | The compiled-in deployment (`manifest:` path or inline keys), for a test to overlay. |
| `run_with` | `async fn run_with<B>(DeploymentBuilder, B) -> Result<ExitStatus>` | Build the builder, then drive the deployment through `Hooks` over a bundle already in hand; nothing connects. |

`main` and `run` use the generated `Backends` bundle. `Hooks`, `manifest`, and `run_with` exist so a test drives the *same* wiring the binary runs — over `omnia_test::host::Backends`, say — without a second `runtime!` declaration in the test tree.
