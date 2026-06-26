# Design: A `wasi:cli` Command Example

> Status: Design proposal — adds an `examples/cli` deployment whose guest implements the
> `wasi:cli/command` world (a small set of argv-dispatched subcommands) and a host that
> invokes its `wasi:cli/run` export once and exits. Surfaces (and works around) two floor
> gaps: nothing invokes `wasi:cli/run` today, and the fixed per-store WASI context never
> populates guest argv.
>
> Owns: `examples/cli/{guest.rs,runtime.rs,README.md}` and their `examples/Cargo.toml`
> entries. Depends: the landed `RegistryBuilder` / `Compiled` / `Registry` pipeline, the
> `Runtime` trait, `StoreBase`, `#[derive(StoreContext)]`, and `wasmtime-wasi`'s p2
> command bindings (`wasmtime_wasi::p2::bindings::{Command, CommandPre}`).

## 1. Abstract

Every existing example is a *reactor*: a `cdylib` guest that exports a handler interface
(`wasi:http/incoming-handler`, `wasi:messaging/incoming-handler`, …) which a long-lived
host `Server` drives on each inbound event. This design adds the missing shape — a
*command*: a guest that implements the `wasi:cli/command` world (`export wasi:cli/run`),
reads `wasi:cli/environment` arguments, dispatches a small set of subcommands, writes to
stdout/stderr, and exits.

The guest half is well supported with no new floor code. The host half is not: nothing in
`omnia` invokes a `wasi:cli/run` export, and the fixed per-store WASI context wires
stdio + env but never sets argv. The example therefore ships a small hand-written host
(modelled on `crates/omnia/tests/linking.rs`) that injects argv and calls `wasi_cli_run`,
plus a documented path to promoting this into a first-class `wasi:cli` trigger if the need
recurs.

## 2. Problem

### 2.1 What already works (the imports half)

`wasi:cli/*` imports are linked unconditionally by the base linker, so any command guest's
imports resolve out of the box (`crates/omnia/src/create.rs` lines 157–162):

```rust
let mut linker = Linker::new(&engine);
wasmtime_wasi::p2::add_to_linker_async(&mut linker)?;
wasmtime_wasi::p3::add_to_linker(&mut linker)?;
```

And the per-store WASI context inherits env + stdin and routes stdout/stderr to the host
process (`crates/omnia/src/store.rs` lines 52–58):

```rust
let wasi = WasiCtxBuilder::new()
    .inherit_env()
    .inherit_stdin()
    .stdout(tokio::io::stdout())
    .stderr(tokio::io::stderr())
    .build();
```

So a guest's `println!`, `eprintln!`, `std::env::vars()`, and stdin reads all work today.

### 2.2 What is missing (the export half)

| Gap | Detail |
|---|---|
| **No `wasi:cli/run` invoker** | Every trigger (`wasi-http`, `wasi-messaging`, `wasi-websocket`) is a `Server` that probes for a handler export and serves *forever*. A command is the opposite: invoke `run` once, then exit. `Server::run` even defaults to a no-op `async { Ok(()) }` (`crates/omnia/src/traits.rs`), so no built-in path calls `run`. |
| **No guest argv** | §2.1 shows the WASI builder sets env + stdio but **not** `.args(...)`. A guest calling `std::env::args()` / `wasi:cli/environment.get-arguments()` sees an empty list. "A small set of commands" dispatched on argv is therefore impossible without injecting args. |
| **CLI consumes process args** | `omnia::Command::Run { wasm, config }` (`crates/omnia/src/lib.rs`) captures no trailing args, so `omnia run x.wasm greet Ada` has nowhere to put `greet Ada`. The `runtime!`-generated `main` cannot forward guest argv. |

### 2.3 Reactor vs command target shape

Every guest example declares `crate-type = ["cdylib"]` (a reactor). A `wasi:cli/command`
component is a *binary* (`fn main`); `wasm32-wasip2` maps a Rust `main` onto
`wasi:cli/run` automatically. The example's guest target must therefore **not** set
`crate-type`, and — because `cargo build` compiles examples for the host too — must guard
`main` so the host build still has one.

## 3. Goals

1. **A runnable command example**: a guest implementing `wasi:cli/command` with a handful
   of subcommands (`greet`, `add`, `env`), driven argv-first.
2. **A minimal host that invokes `wasi:cli/run`** and exits with the guest's status,
   reusing the `omnia` registry/engine/pooling pipeline rather than raw wasmtime.
3. **Solve argv injection** with a localized, documented mechanism.
4. **Match the two-step example workflow** (`build <name>-wasm` → `run <name>`) and the
   existing `guest.rs` / `runtime.rs` / `README.md` layout.
5. **No floor behaviour change** in the recommended scope — the example is self-contained;
   `cargo make ci` stays green.

## 4. Non-goals

- A first-class, reusable `wasi:cli` trigger crate (`omnia-wasi-cli`) wired through
  `runtime!`'s `hosts` map. Sketched in [§9 Alternatives](#9-alternatives-considered) as
  the path if commands become common; out of scope for one example.
- Changing `RegistryBuilder`, `Compiled`, the guest registry, or dispatch machinery.
- p3 command bindings. The command world is exercised through p2
  (`wasmtime_wasi::p2::bindings`), which the base linker already provides.
- Multi-guest command routing (`omni.toml` `[[route.*]]`). The example is the single-file
  shorthand: one guest, taken directly.

## 5. Design

### 5.1 Guest — a `wasi:cli/command` binary

A plain Rust binary, dispatched on `std::env::args()`. No `wit-bindgen`: the `wasm32-wasip2`
target emits the command component and maps std I/O onto `wasi:cli/*`.

```rust
// examples/cli/guest.rs

#[cfg(target_arch = "wasm32")]
fn main() {
    let args: Vec<String> = std::env::args().collect();
    // args[0] is the program name; args[1] is the subcommand.
    match args.get(1).map(String::as_str) {
        Some("greet") => {
            let who = args.get(2).map(String::as_str).unwrap_or("world");
            println!("Hello, {who}!");
        }
        Some("add") => {
            let sum: i64 = args[2..].iter().filter_map(|a| a.parse::<i64>().ok()).sum();
            println!("{sum}");
        }
        Some("env") => {
            for (key, value) in std::env::vars() {
                println!("{key}={value}");
            }
        }
        Some(other) => {
            eprintln!("unknown command: {other}");
            std::process::exit(2);
        }
        None => {
            eprintln!("usage: <greet|add|env> [args...]");
            std::process::exit(1);
        }
    }
}

// A binary example needs a `main` when cargo builds it for the host target.
#[cfg(not(target_arch = "wasm32"))]
fn main() {}
```

A nonzero `std::process::exit` (or a panic) surfaces as `Err(())` from `wasi:cli/run`; the
host maps that to a nonzero process exit (§5.3).

### 5.2 Host — invoking `wasi:cli/run`

The closest landed pattern is `crates/omnia/tests/linking.rs`: a minimal `StoreCtx`, a
hand-rolled `Runtime`, and a helper that pulls a guest from the registry and calls an
export (lines 49–103). The example reuses that shape, with two changes — inject argv in
`store()`, and invoke through the typed p2 command bindings rather than `get_func("run")`
(the `run` func lives inside the versioned `wasi:cli/run@…` instance, not at the root, so
`CommandPre` does the export lookup).

```rust
// examples/cli/runtime.rs  (host-only)
#![cfg(not(target_arch = "wasm32"))]

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use omnia::wasmtime_wasi::WasiCtxBuilder;
use omnia::wasmtime_wasi::p2::bindings::CommandPre;
use omnia::{Registry, RegistryBuilder, Runtime, StoreBase, StoreContext};

#[derive(StoreContext)]
struct CliCtx {
    #[base]
    base: StoreBase,
}

#[derive(Clone)]
struct CliRuntime {
    registry: Arc<Registry<CliCtx>>,
    args: Arc<Vec<String>>, // guest argv; args[0] is the program name
}

impl Runtime for CliRuntime {
    type StoreCtx = CliCtx;

    fn store(&self) -> CliCtx {
        // StoreBase omits guest argv (store.rs §2.1), so rebuild `wasi` with `.args(...)`.
        // `base.wasi` is a public field, so the override is local — no floor change.
        let mut base = StoreBase::new(self.options(), Arc::new(self.clone()));
        base.wasi = WasiCtxBuilder::new()
            .inherit_env()
            .inherit_stdin()
            .stdout(tokio::io::stdout())
            .stderr(tokio::io::stderr())
            .args(&self.args)
            .build();
        CliCtx { base }
    }

    fn registry(&self) -> &Registry<Self::StoreCtx> {
        &self.registry
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // The example owns its argv (it is not the `omnia` CLI): `cargo run --example cli --
    // <wasm> greet Ada`. The host takes argv[1] as the component path and forwards the
    // rest as the guest's argv, prepending a program name at index 0.
    let mut argv = std::env::args().skip(1);
    let wasm = argv.next().context("usage: cli <wasm> [guest args...]")?;
    let mut guest_args = vec!["cli".to_string()];
    guest_args.extend(argv);

    let registry = RegistryBuilder::new()
        .wasm(PathBuf::from(wasm))
        .compile::<CliCtx>()
        .await?
        .build()?;
    let runtime = CliRuntime { registry: Arc::new(registry), args: Arc::new(guest_args) };

    // Single-file shorthand => exactly one guest. (For many guests, iterate
    // `registry.guests()` and pick the first where `CommandPre::new(..)` is Ok — the same
    // capability-probe idea `wasi-http`'s `TriggerRouter::build` uses for handler exports.)
    let guest = runtime.registry().guests().next().context("a guest is registered")?;

    let mut store = runtime.build_store(runtime.store());
    let pre = CommandPre::new(guest.instance_pre().clone())?;
    let command = pre.instantiate_async(&mut store).await?;
    match command.wasi_cli_run().call_run(&mut store).await? {
        Ok(()) => Ok(()),
        Err(()) => std::process::exit(1),
    }
}
```

`build_store` still applies the epoch deadline, fuel budget, and memory limiter
(`crates/omnia/src/traits.rs`), so the command runs under the same per-guest guards as a
triggered handler. `CommandPre::new` re-uses the registry's `InstancePre`, front-loading
the `run` export lookup; it bypasses `Runtime::instantiate`, so the
`instantiation_duration_us` / `pool_instantiation_errors` metrics that
`Runtime::instantiate` records are not emitted for this path (acceptable for an example;
see [§11 Open questions](#11-open-questions)).

### 5.3 Argv injection

The crux. Two viable mechanisms:

| Option | Mechanism | Cost |
|---|---|---|
| **A (chosen)** | Override `base.wasi` in the example's `store()` with a `WasiCtxBuilder` that adds `.args(...)`. | Localized to the example; `StoreBase.wasi` is already `pub`. Duplicates the four inherit/stdio lines. No floor change. |
| **B (upstream)** | Add `StoreBase::with_args(options, host_dispatch, args)` (or an `args: Vec<String>` field) so any future command host sets argv without rebuilding `wasi`. | Touches `crates/omnia/src/store.rs` and its doc/tests. Justified only once a second command consumer exists. |

This design takes **A** to keep the change confined to `examples/`. If a `wasi:cli`
trigger lands later (§9), it should take **B** so argv has a documented home.

Host-level passthrough is intentionally the example's own argv parsing, not the `omnia`
CLI. Generalizing `omnia run x.wasm -- greet Ada` would require `omnia::Command::Run` to
grow a trailing `args: Vec<String>` and the runtime to thread it into `store()` — an
upstream change deferred with **B**.

## 6. Constraints

### 6.1 Command is a binary, not a reactor

The guest target omits `crate-type` (a binary) rather than `cdylib` (a reactor). Because
`cargo build`/`cargo test` compile examples for the host triple too, the guest file guards
the real entrypoint with `#[cfg(target_arch = "wasm32")]` and supplies an empty
`#[cfg(not(target_arch = "wasm32"))] fn main() {}` — the mirror image of how
`runtime.rs` files stub the wasm side with `cfg_if`.

### 6.2 Exit semantics

`call_run` returns `Result<Result<(), ()>>`: the outer `Result` is a host-side trap, the
inner is the guest's `wasi:cli/run` result. `Ok(Ok(()))` is success; `Ok(Err(()))` (the
guest exited nonzero or trapped its run) maps to `std::process::exit(1)`; the outer `Err`
propagates as an `anyhow` error. This mirrors the canonical wasmtime command host.

### 6.3 p2, not p3

`Command` / `CommandPre` are p2 bindings and need the p2 `wasi:cli/*` imports, which
`add_to_linker_async` provides (§2.1). A p2 command's `call_run().await` drives directly
on the multi-threaded runtime (as `tests/linking.rs` calls `call_async` without
`run_concurrent`); the p3 `run_concurrent` dance the HTTP server uses is not required.

## 7. Cargo wiring

```toml
# examples/Cargo.toml
[[example]]
name = "cli-wasm"
path = "cli/guest.rs"
# no crate-type: a binary -> wasi:cli/command component

[[example]]
name = "cli"
path = "cli/runtime.rs"
```

Build + run (note cargo's hyphen→underscore filename rule):

```bash
cargo build --example cli-wasm --target wasm32-wasip2
cargo run --example cli -- ./target/wasm32-wasip2/debug/examples/cli_wasm.wasm greet Ada
cargo run --example cli -- ./target/wasm32-wasip2/debug/examples/cli_wasm.wasm add 2 3 4
cargo run --example cli -- ./target/wasm32-wasip2/debug/examples/cli_wasm.wasm env
```

## 8. Before → after

There is no "before"; this is additive. The shape relative to existing examples:

| Aspect | Reactor examples (`http`, `messaging`, …) | This command example (`cli`) |
|---|---|---|
| Guest target | `crate-type = ["cdylib"]` | binary (`fn main`, cfg-guarded) |
| Guest export | `wasi:*/incoming-handler` via `export!` | `wasi:cli/run` via `fn main` |
| Host | `omnia::runtime!({ hosts: {…} })` | hand-written `main` + `Runtime` impl |
| Host lifecycle | `Server::run` loops forever | invoke `run` once, exit |
| Argv | n/a | injected via `store()` override |
| Lines of host code | ~12 (macro) | ~50 |

## 9. Alternatives considered

### 9.1 Standalone wasmtime-wasi host (no `omnia`)

Build an `Engine` + `Linker`, `add_to_linker_async`, a `WasiCtx` with `.args().inherit_stdio()`,
then `CommandPre::new(linker.instantiate_pre(&component)?)` + `call_run`. Fewest moving
parts, but it bypasses the whole runtime (registry, pooling, options, telemetry) — an
example in this repo that never touches `omnia` is misleading. Rejected as the primary
form; it is essentially what §5.2 reduces to if `RegistryBuilder` is dropped.

### 9.2 First-class `wasi:cli` trigger (`omnia-wasi-cli` crate)

Add a host crate with `impl Server<R> for WasiCli` whose `run()` probes the registry for a
`wasi:cli/run` exporter (analogous to `wasi-http`'s `TriggerRouter::build`), invokes it,
and returns `Ok(())` so `omnia::serve`'s `try_join_all` completes and the process exits.
Wiring it through `runtime!({ hosts: { WasiCli: … } })` fights two conventions:

- The macro derives a `StoreCtx` field and view-macro path per host (`wasi_ident`:
  `WasiCli` → `omnia_wasi_cli`, expecting `omnia_wasi_cli::omnia_wasi_view!`) and a
  `Backend` per host (`crates/runtime-macro/src/expand.rs` lines 154–205). `wasi:cli`
  needs neither a backend nor host state — its imports are linked by the base linker and
  it holds no view.
- Argv still has no home without §5.3 option **B**.

This is the right move *if commands become a common deployment shape* (then take option
**B**, and consider letting the macro list server-only hosts). It is too much machinery to
justify for a single example, so it is documented here rather than built.

### 9.3 Reactor guest exporting `wasi:cli/run` via `wit-bindgen`

Keep `crate-type = ["cdylib"]` and `impl exports::wasi::cli::run::Guest`, reading args via
`wasi::cli::environment::get_arguments()`. Avoids the binary/cfg dance but adds a
`wit-bindgen::generate!` of the command world and loses `std::env`/`println!`
ergonomics. The `fn main` binary (§5.1) is the canonical "implements `wasi:cli`" form;
this is noted only for authors who prefer an explicit export.

## 10. Implementation plan

1. **Add `examples/cli/guest.rs`** — the binary command of §5.1.
2. **Add `examples/cli/runtime.rs`** — the hand-written host of §5.2.
3. **Add the two `[[example]]` entries** to `examples/Cargo.toml` (§7); the guest entry
   has **no** `crate-type`.
4. **Add `examples/cli/README.md`** with the build/run commands and the argv convention.
5. **Verify**: `cargo build --example cli-wasm --target wasm32-wasip2`, then the three
   `cargo run --example cli -- …` invocations produce the expected stdout; `cargo make ci`
   stays green.

Steps 1–4 are independent of every other crate. No floor file changes under the chosen
scope (option **A**).

## 11. Open questions

- **Argv home.** Ship option **A** (example-local `base.wasi` override) now, or pull
  `StoreBase::with_args` (option **B**) upstream pre-emptively? **A** unless §9.2 is on the
  near-term roadmap.
- **Instantiation metrics.** `CommandPre::instantiate_async` skips `Runtime::instantiate`,
  so the example emits no instantiation metrics. Acceptable for an example; a real trigger
  (§9.2) should route through `Runtime::instantiate` and then bind the typed `run`.
- **Multi-guest commands.** Out of scope here, but if added: which guest answers when two
  export `wasi:cli/run`? The `wasi-http` precedent (single exporter ⇒ catch-all; many ⇒
  require `[[route.*]]`) suggests a `[[route.cli]]`-style table keyed by the first argv
  token.
- **stdin.** The example inherits host stdin; a future `env`/filter subcommand reading
  stdin would exercise that path. Not needed for the initial three commands.

## 12. Acceptance criteria

1. `examples/cli/guest.rs` builds to a `wasi:cli/command` component under
   `wasm32-wasip2` and implements `greet`, `add`, and `env`.
2. `examples/cli/runtime.rs` loads that component via `RegistryBuilder`, injects argv, and
   invokes `wasi:cli/run`, exiting with the guest's status.
3. The three `cargo run --example cli -- …` invocations (§7) print the expected output;
   an unknown subcommand exits nonzero.
4. The guest target declares no `crate-type` and compiles for the host triple (via the cfg
   stub) so `cargo build` / `cargo test` succeed.
5. No floor crate is modified; `cargo make ci` stays green.

## 13. Risks and invariants

- **No floor change (chosen scope).** All new code lives in `examples/`. The only contact
  with floor internals is reading the `pub` `StoreBase.wasi` field; if that field's
  construction policy changes, the example's `store()` override must track it (or move to
  option **B**).
- **Law 2 preserved.** `wasi:cli` is a standard WASI interface and `GuestId` stays opaque;
  the example carries no consumer vocabulary, and `host_dispatch` remains the generic
  host→guest seam (unused by a command).
- **Instance-per-call unchanged.** The command is instantiated fresh on a new store and
  discarded, exactly like a triggered handler; `build_store` guards still apply.
- **Process lifetime.** The host exits when `run` returns — there is no server loop. If a
  command host is ever combined with a long-lived trigger in one process, `omnia::serve`'s
  `try_join_all` would block on the trigger; a command is therefore its own deployment.

## 14. References

- [runtime-interface.md](runtime-interface.md) — `StoreBase`, `#[derive(StoreContext)]`,
  `omnia::serve`, and the defaulted `Runtime` methods this example builds on.
- `crates/omnia/tests/linking.rs` — the hand-written `Runtime` + call-an-export pattern
  §5.2 mirrors.
- `crates/wasi-http/src/host/server.rs` — `TriggerRouter::build` capability probe, the
  model for multi-guest command selection (§9.2, §11).
- `crates/omnia/src/store.rs`, `crates/omnia/src/create.rs` — the linked imports and the
  argv gap (§2).
