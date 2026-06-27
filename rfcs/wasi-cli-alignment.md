# Design: Aligning `wasi:cli` — a `cdylib` Guest and a One-Shot Trigger Host

> Status: Accepted. `examples/cli` diverges from every other example on **two** axes: its guest is a binary (not a `cdylib` reactor) and its host is hand-written (not macro-wired). This design erases both **without forking the floor**: the guest becomes a `cdylib` exporting `wasi:cli/run` via `wasip3`, and `wasi:cli` becomes a first-class **trigger** — a `WasiCli` host that implements `Host` + `Server` exactly like `WasiHttp`, builds a `TriggerRouter`, instantiates instance-per-call, and runs through the existing `omnia::serve` lifecycle, with the `runtime!` macro unchanged. The only thing that differs from a long-lived trigger is the lifecycle *tail*: a command fires once and its response is an exit code.
>
> This is the **Level-1 convergence** target (deployment-level): the same guest-handler model, the same `Server` trait, the same `TriggerRouter`, the same `runtime!` macro, and the same manifest serve every event source — so moving a guest from a CLI trigger today to an HTTP / messaging / websocket / wRPC trigger tomorrow is a *wiring* change, not a *path* change. Guest-level convergence (one uniform handler the floor adapts every event into) is **Level 2** and explicitly out of scope; it slots in cleanly later precisely because the trigger machinery is unified now.
>
> Owns: the committed design for aligning `examples/cli` on both axes. Touches: `examples/cli/{guest.rs,runtime.rs,README.md}`, `examples/Cargo.toml`, a new host-only crate `crates/wasi-cli`, `crates/omnia/src/{runtime.rs,lib.rs,routing.rs}` (the `ExitStatus` newtype and the optional `cli` route table), and `crates/host-macros/src/{runtime.rs,expand.rs,runtime_derive.rs}` (a backend-optional host grammar, an argv field, and a one-shot `main` tail). Depends: the landed `StoreBase::builder` argv setter, the `Runtime` / `StoreContext` derives, `omnia::serve` + `TriggerRouter` (`crates/omnia/src/{runtime.rs,routing.rs}`), the base linker's `wasi:cli` import linking (`crates/omnia/src/create.rs`), `wasmtime-wasi`'s p2 `Command` / `CommandPre` bindings, and [`wasip3`](https://docs.rs/wasip3) guest export macros (Bytecode Alliance [`wasi-rs`](https://github.com/bytecodealliance/wasi-rs)).

## 1. Abstract

`examples/cli` is the repo's only *command* (a guest that exports `wasi:cli/run`, invoked once); every other example is a *reactor* (a `cdylib` exporting a handler that a long-lived `Server` drives on each inbound event). To keep the first command self-contained it took the two localized shortcuts — a binary guest and a hand-written host — which leave it reading nothing like the deployments a newcomer learns first.

This design removes both divergences **by treating a command as one more trigger**, not as a separate species:

- **[§4 Guest](#4-guest-alignment--cdylib-reactor)** — rewrite the guest as a `cdylib` reactor exporting `wasi:cli/run` via `wasip3::cli::command::export!` (the same guest-SDK layer HTTP uses), so it shares the target shape, filename rule, and cfg convention of every other guest.
- **[§5 Host](#5-host-alignment--wasicli-as-a-one-shot-trigger)** — add a `WasiCli` host that is a trigger in the same sense `WasiHttp` is: it implements `Host` (a no-op linker — `wasi:cli`'s *imports* are ambient, and a trigger *drives an export* rather than linking an import) and `Server` (a one-shot `run` that capability-probes `wasi:cli/run` through `TriggerRouter`, instantiates instance-per-call through `Runtime::instantiate`, calls `call_run`, and reports an `ExitStatus`). It is wired by the **existing** `runtime!` macro and run by the **existing** `omnia::serve`.

The unifying idea: **every guest is a `cdylib` exporting a handler; every trigger is a `Server` selected by a `TriggerRouter` and driven by `omnia::serve`.** A command's handler is `wasi:cli/run`; its trigger is `WasiCli`; its per-invocation response is an exit code, just as an HTTP trigger's response is a status code and body. The "command vs. server" distinction collapses into a single, smaller fact — **a command is a one-shot trigger whose transport is the process** — which the `Server` abstraction already encapsulates.

Crucially, the command path reuses `Server`, `TriggerRouter`, and `omnia::serve` rather than a command-only lifecycle running parallel to them. A parallel path would erase the *surface* divergence (example shape) only to introduce a deeper *architectural* one — a "command world" that bypasses the floor's trigger machinery — which is exactly the divergence Level-1 convergence forbids. [§8](#8-alternatives-considered-and-rejected) weighs that alternative.

## 2. Problem — where `examples/cli` diverges

### 2.1 Guest divergences

`examples/cli/guest.rs` is a plain Rust binary; the `wasm32-wasip2` target maps its `fn main` onto `wasi:cli/run` and std I/O onto `wasi:cli/*`. This makes it the odd one out:

| Divergence | `examples/cli` (binary) | Every reactor guest (`cdylib`) |
| --- | --- | --- |
| Cargo target | the only `[[example]]` with **no** `crate-type` (`examples/Cargo.toml`) | `crate-type = ["cdylib"]` |
| Host-target build | a `#[cfg(target_arch = "wasm32")]` real `main` + an empty `#[cfg(not(...))] fn main() {}` stub (`examples/cli/guest.rs` lines 19–51) | `#![cfg(target_arch = "wasm32")]` on the whole file; no host stub |
| Component filename | keeps its hyphen: `cli-wasm.wasm` (named after the target as-is) | the crate-name rule forces `*_wasm.wasm`, e.g. `http_wasm.wasm` |
| Export-side SDK | none — the `wasm32-wasip2` rustc std bridge maps `main` → `wasi:cli/run` | `wasip3::cli::command::export!` (same layer as HTTP's `wasip3::http::service::export!`) |

### 2.2 Host divergences

`examples/cli/runtime.rs` is a hand-rolled `Runtime` + `main` (~50 lines). The reactor examples are instead ~12 lines of `omnia::runtime!`. The divergences:

| Divergence | `examples/cli` (hand-written) | Every reactor host (`runtime!`) |
| --- | --- | --- |
| Host body | a `#[derive(StoreContext)]` ctx, a `Runtime` impl, and a `#[tokio::main]` | `omnia::runtime!({ main: true, hosts: { … } })` |
| Lifecycle | instantiate, `call_run`, map exit code, exit (`runtime.rs` lines 88–105) | `omnia::serve` joins long-lived servers (`crates/omnia/src/runtime.rs` line 109) |
| argv | injected via `StoreBase::builder().args(&args)` (`runtime.rs` lines 49–54) | no path exists; the generated `store()` omits `.args` |
| Routing | a hand-inlined `CommandPre::new(..).is_ok()` probe over `guests().next()` (`runtime.rs` lines 87–90) | `TriggerRouter::build(.., probe)` capability-probes + routes (`wasi-http`/`wasi-messaging` `server.rs`) |
| Exit status | a guest `i32` honoured via `std::process::exit` *inside* the host body | none — servers do not return a status |

Both halves are *correct*; they are just bespoke. The hand-written probe in particular is a one-off reimplementation of `TriggerRouter` — the same capability-probe-then-select that every real trigger already does generically.

## 3. A command is a one-shot trigger, not a separate species

It is tempting to treat "command" and "server" as different species — five axes seem to separate them, and they could motivate a `server!`/`cli!` macro split. But every one of those axes is a property of the **transport** — one-shot vs. long-lived I/O — which the `Server` abstraction is meant to *encapsulate*, not *fork on*:

| Axis | Read as "command ≠ server" | Read as "CLI is a one-shot trigger" |
| --- | --- | --- |
| Lifecycle | a command does "one invoke, then exit"; a server `try_join_all`s forever | "fires once" is what a CLI *transport* does, as "loops forever" is what an HTTP *transport* does. `serve`'s `try_join_all` already completes for a sole one-shot server (its future resolves after the single invocation). |
| Exit status | a command returns an `i32`; a server returns nothing | the exit code is the CLI trigger's **per-invocation response** — the analog of an HTTP status code. Triggers already produce a response and deliver it over their transport (HTTP via a `oneshot` to hyper, `crates/wasi-http/src/host/server.rs` lines 168–239). CLI's transport is the process; its response is the exit code. It never needed to ride `Server::run`'s `Result<()>`. |
| argv | a command consumes argv; a server has none | argv is the CLI trigger's **input** — the analog of the HTTP `Request`. Already solved by `StoreBase::builder().args` (`crates/omnia/src/store.rs` lines 39–49). |
| Backend / view | the macro parser requires a backend per host | a self-imposed grammar constraint (`crates/host-macros/src/runtime.rs` lines 99–106), relaxed once in §5.4 (backend-optional hosts) for the *one* macro. |
| Background tasks | epoch/pool tasks are vestigial for a one-shot | epoch interruption is *useful* for a one-shot (it lets a guest deadline fire); `sample_pool` is already a no-op when disabled (`crates/omnia/src/runtime.rs` lines 48–51). `serve` spawns both regardless and they die with the process. |

The decisive evidence that this is the floor's grain: **HTTP, messaging, and websocket already coexist as triggers in one `runtime!`, one `serve`, and one `TriggerRouter`** — `examples/messaging/runtime.rs` wires four hosts at once, and both `wasi-http` and `wasi-messaging` build via `TriggerRouter::build(state.registry(), trigger, table, probe)`. CLI is the *only* event source being singled out for a parallel path. Folding it into the same machinery is the alignment; carving it out is the divergence.

## 4. Guest alignment — `cdylib` reactor

> Make the guest a `cdylib` reactor exporting `wasi:cli/run`, so it shares the target shape, filename rule, and cfg convention of every other guest.

### 4.1 Target idiom

Reactor guests split into two binding layers, and **not every binding comes from an `omnia-wasi-*` crate**:

| Layer | Source | Role in reactor examples |
| --- | --- | --- |
| **Standard WASI export** | [`wasip3`](https://docs.rs/wasip3) (already a workspace dep for HTTP) | what the component *exports* to the host — e.g. `wasip3::http::service::export!` in every HTTP reactor |
| **Extension WASI import** | `omnia-wasi-*` `guest.rs` (`wit_bindgen::generate!`) | what the guest *calls* on the host — e.g. `omnia_wasi_keyvalue::store` |
| **Ergonomics** | helpers in `omnia-wasi-*` | e.g. `omnia_wasi_http::serve` bridging axum ↔ `wasip3` HTTP types |

There is **no `omnia-wasi-cli` guest module**, and one is not required: Bytecode Alliance [`wasi-rs`](https://github.com/bytecodealliance/wasi-rs) ships pre-generated guest bindings and an export macro for `wasi:cli/command`, parallel to the HTTP export macros. `wasmtime-wasi` is the **host** side only — it does not generate guest code. The closest precedent is therefore `examples/http/guest.rs`:

```rust
// examples/http/guest.rs — the export-side idiom to mirror
#![cfg(target_arch = "wasm32")]

use wasip3::exports::http::handler::Guest;
use wasip3::http::types::{ErrorCode, Request, Response};

struct HttpGuest;
wasip3::http::service::export!(HttpGuest);

impl Guest for HttpGuest { /* ... */ }
```

The aligned cli guest is the same export-side shape — `wasip3` throughout, reusing the workspace dependency the HTTP examples already carry:

```rust
// examples/cli/guest.rs (aligned) — wasip3, the same guest SDK as HTTP
#![cfg(target_arch = "wasm32")]

struct Cli;

impl wasip3::exports::cli::run::Guest for Cli {
    async fn run() -> Result<(), ()> {
        // argv via wasi-rs import bindings, not std::env::args()
        let args = wasip3::cli::environment::get_arguments();
        match args.get(1).map(String::as_str) {
            Some("greet") => {
                let who = args.get(2).map_or("world", |s| s.as_str());
                let msg = format!("Hello, {who}!\n");
                wasip3::cli::stdout::get_stdout().blocking_write_and_flush(msg.as_bytes())?;
            }
            // add / env / unknown ...
            _ => return Err(()),
        }
        Ok(())
    }
}

wasip3::cli::command::export!(Cli);
```

The p2 host invoke path is unaffected: the floor links both p2 and p3 (`crates/omnia/src/create.rs` lines 159–160), and the host's `CommandPre` keys on the presence of `wasi:cli/run`, not on which guest SDK produced the export — the same split the HTTP examples already rely on (`wasip3` guest, mixed p2/p3 host linker).

### 4.2 What it takes

1. **`wasip3` on the `cli-wasm` example target** — already a workspace dep for HTTP; provides `cli::command::export!`, `exports::cli::run::Guest`, and the `wasi:cli/*` import bindings (`environment`, `stdout`, `stderr`, …). No local `wit/` directory, no new dependency.
2. **`wasip3::cli::command::export!` + `impl exports::cli::run::Guest`** (`async fn run`).
3. **Dispatch over wasi-rs import bindings**: `wasip3::cli::environment::get_arguments()` for argv, `get_environment()` for the `env` subcommand, and `stdout`/`stderr` writes via the generated helpers in place of `println!`/`eprintln!`. A nonzero result becomes `Err(())` (rather than `std::process::exit`).
4. **`examples/Cargo.toml`**: give the `cli-wasm` entry `crate-type = ["cdylib"]`, which also flips its component filename from `cli-wasm.wasm` to `cli_wasm.wasm` — a README/run-command update.

### 4.3 The tradeoff, accepted

The binary form *is* the canonical "write a `wasi:cli` command in Rust": `fn main`, `std::env`, `println!`, and `std::process::exit` all map onto `wasi:cli/*` for free. The `cdylib` form discards that std bridge — the `wasip3::cli::command::export!` macro is [incompatible with the `bin` target](https://docs.rs/wasip3/latest/wasip3/cli/command/macro.export.html) — and routes dispatch through `wasip3::cli::*` import APIs instead. So as a *standalone* artifact the `cdylib` guest is the less idiomatic "how to write a command."

We accept that cost deliberately, because this is a teaching repo and **one consistent guest mental model beats per-example idiom**: every guest is a `cdylib` that exports a handler. The command's handler is `wasi:cli/run`. A reader who has internalised the reactor shape reads the cli guest with no new concepts — the learnability win the binary form forfeits. (If Omnia-specific CLI ergonomics ever emerge, an `omnia-wasi-cli` guest helper analogous to `omnia_wasi_http::serve` can wrap the `wasip3` export; it is not on the critical path.)

## 5. Host alignment — `WasiCli` as a one-shot trigger

> Add a `WasiCli` host that is a trigger like `WasiHttp`: `Host` (no-op linker) + `Server` (one-shot `run` via `TriggerRouter`). Wire it with the existing `runtime!` macro; run it with the existing `omnia::serve`. The only new floor types are an `ExitStatus` newtype and an optional `cli` route table.

### 5.1 The `WasiCli` host — `Host` + `Server`, exactly like `WasiHttp`

`WasiHttp` is two impls: a `Host` that links the `wasi:http` *imports* a guest may call, and a `Server` whose `run` drives the `wasi:http/incoming-handler` *export* (`crates/wasi-http/src/host.rs` lines 19–36). `WasiCli` mirrors that shape. Its `Host` impl is a no-op — and that is *correct, not awkward*: `wasi:cli`'s imports (`environment`, `stdout`, `stderr`, …) are **ambient** (any reactor guest may write to stderr), so they live in the base linker already (`crates/omnia/src/create.rs` lines 159–160); and a trigger **drives an export**, which needs no linking, exactly as `WasiHttp`'s server drives `incoming-handler` without linking it.

```rust
// crates/wasi-cli/src/host.rs  (new, host-only crate; the guest uses wasip3 directly per §4.1)
use std::sync::{Arc, OnceLock};

use anyhow::{Result, bail};
use omnia::wasmtime_wasi::I32Exit;
use omnia::wasmtime_wasi::p2::bindings::Command;
use omnia::{ExitStatus, Host, Runtime, Server, TriggerRouter};
use omnia::wasmtime_wasi::p2::bindings::CommandPre;
use wasmtime::component::Linker;
use wasmtime_wasi::WasiView;
use wrpc_wasmtime::WrpcView;

/// Host-side trigger for `wasi:cli`. Drives the `wasi:cli/run` export of the
/// sole command-capable guest exactly once and reports its exit status.
#[derive(Debug, Clone, Default)]
pub struct WasiCli {
    /// The one-shot's result, read by the generated `main` at the process
    /// boundary (see §5.2). The status rides this side channel because
    /// `Server::run` / `omnia::serve` return `Result<()>` and discard each
    /// server's value — the same way `WasiHttp` delivers its response out of
    /// band (over the socket) rather than through `run`'s return type.
    exit: Arc<OnceLock<ExitStatus>>,
}

impl WasiCli {
    #[must_use]
    pub fn new(exit: Arc<OnceLock<ExitStatus>>) -> Self {
        Self { exit }
    }
}

impl<T> Host<T> for WasiCli
where
    T: WasiView + 'static,
{
    // `wasi:cli`'s imports are ambient (base linker); a trigger drives an
    // export, so there is nothing to add here. Cf. the no-op `Server` default
    // (`crates/omnia/src/traits.rs` lines 122–131).
    fn add_to_linker(_linker: &mut Linker<T>) -> Result<()> {
        Ok(())
    }
}

impl<R> Server<R> for WasiCli
where
    R: Runtime,
    R::StoreCtx: WasiView + WrpcView + 'static,
{
    async fn run(&self, state: &R) -> Result<()> {
        // (1) Capability probe + routing — the same `TriggerRouter` HTTP and
        // messaging use. A guest is command-capable exactly when its
        // `wasi:cli/run` export resolves (`CommandPre::new` is `Ok`). The `cli`
        // route table is empty today, so a sole exporter is the catch-all,
        // zero is inert, and ">1 with no routes" is the same ambiguity error
        // HTTP raises (`crates/omnia/src/routing.rs` lines 172–196).
        let routing = TriggerRouter::build(
            state.registry(),
            "cli",
            state.registry().routes().cli().clone(),
            |pre| CommandPre::new(pre.clone()).map(|_| ()), // I = () capability marker
        )?;
        if routing.is_inert() {
            tracing::info!("no guest exports wasi:cli/run; cli trigger inert");
            return Ok(()); // nothing to drive; `serve` then completes
        }
        let Some((guest_id, _)) = routing.catch_all() else {
            // >1 command-capable guest but no `[[route.cli]]` to disambiguate.
            // Multi-command routing is §5.6; until then this is a clean error.
            bail!("multiple wasi:cli/run guests but no [[route.cli]] to disambiguate");
        };
        let guest = state.registry().get(guest_id).expect("a capable guest is registered");

        // (2) Instance-per-call, *through* `Runtime::instantiate` — so a command
        // records the same `instantiation_duration_us` / `pool_instantiation_errors`
        // metrics every trigger does (which the hand-written host skips). argv is
        // already in the store via `StoreBase::builder().args` (§5.3).
        let mut store = state.build_store(state.store());
        let instance = state.instantiate(guest.instance_pre(), &mut store).await?;
        let command = Command::new(&mut store, &instance)?;

        // (3) Invoke once; map the result to an `ExitStatus`. A guest
        // `process::exit` / panic surfaces as `I32Exit`, not `Ok(Err(()))`
        // (the shape the wasmtime CLI's runner uses, mirrored by today's
        // hand-written host, `examples/cli/runtime.rs` lines 96–105).
        let status = match command.wasi_cli_run().call_run(&mut store).await {
            Ok(Ok(())) => ExitStatus::SUCCESS,
            Ok(Err(())) => ExitStatus::from(1),
            Err(error) => match error.downcast_ref::<I32Exit>() {
                Some(exit) => ExitStatus::from(exit.0),
                None => return Err(error), // a real host trap propagates through `serve`
            },
        };

        // (4) Hand the response to the boundary and complete. `serve`'s
        // `try_join_all` resolves here (this is the only server in a command
        // deployment — see §5.5), and the generated `main` reads the cell.
        let _ = self.exit.set(status);
        Ok(())
    }
}
```

Compared with `wasi-http/src/host/server.rs`, the *shape* is identical — probe via `TriggerRouter::build`, bail on inert, resolve, `state.build_store` → `state.instantiate` → load typed bindings → invoke. The differences are exactly the two transport facts from §3: the "accept loop" is a single iteration, and the response (an `ExitStatus`) is delivered to the process rather than a socket.

`ExitStatus` is a thin floor newtype over the guest's `i32`, exported from `omnia` (`crates/omnia/src/runtime.rs`, beside `serve`):

```rust
pub struct ExitStatus(i32);

impl ExitStatus {
    pub const SUCCESS: Self = Self(0);
    #[must_use] pub const fn code(self) -> i32 { self.0 }
}
impl From<i32> for ExitStatus { fn from(c: i32) -> Self { Self(c) } }
// The OS exit status is a `u8`; exotic `i32` codes truncate, matching today's
// `std::process::exit` behaviour. `main` returns `ExitCode` (or calls
// `process::exit(status.code())` for full `i32` fidelity).
impl From<ExitStatus> for std::process::ExitCode {
    fn from(s: ExitStatus) -> Self { Self::from(s.0 as u8) }
}
```

### 5.2 The one-shot / exit-code seam — against the current `serve` / `TriggerRouter`

Neither `omnia::serve` nor `TriggerRouter` changes. The seam is two observations about the *current* signatures:

1. **One-shot rides `serve` unchanged.** `serve(runtime, servers)` ends with `try_join_all(servers)` (`crates/omnia/src/runtime.rs` line 109). `WasiCli::run`'s future resolves after its single invocation, so for a command deployment — whose `servers` vec holds exactly that one future (§5.5) — `try_join_all` resolves and `serve` returns `Ok(())`. The epoch/pool background tasks `serve` spawns are detached and die with the process. No `serve` variant, no `omnia::run`, is needed.

2. **The exit code rides a side channel, read at the boundary.** `serve` returns `Result<()>` and `try_join_all` discards each server's `()`, so the `ExitStatus` cannot ride the lifecycle `Result` — and it should not, any more than an HTTP 500 rides `WasiHttp::run`'s return type. Instead `WasiCli` carries an `Arc<OnceLock<ExitStatus>>`; `run` sets it; the generated `main` (which constructed the cell and handed a clone to `WasiCli::new`) reads it *after* `serve` returns and performs the single `process::exit` **at the boundary**. The process exit therefore never happens inside a `Server`, and needs no second macro to keep it there.

The generated command-shaped `main` and its `runtime::run` (the only parts the macro emits differently for a command — see §5.4):

```rust
// generated by `omnia::runtime!({ main: true, hosts: { WasiCli } })`
mod runtime {
    // The cli counterpart of the server `runtime::run`: same compile + Context +
    // host-link + backend-connect, then `serve` — but returns the one-shot's
    // status read from the cell instead of `Result<()>`.
    pub async fn run(
        wasm: Option<PathBuf>, config: Option<PathBuf>, args: Vec<String>,
    ) -> Result<ExitStatus> {
        let compiled = omnia::RegistryBuilder::new()
            .wasm(wasm)
            .config(config)
            .compile::<StoreCtx>()
            .await?;
        let exit = Arc::new(OnceLock::new());
        let run_state = Context::new(compiled, Arc::new(args)).await?; // argv → store
        let servers: Vec<BoxFuture<'_, Result<()>>> =
            vec![Box::pin(WasiCli::new(exit.clone()).run(&run_state))];
        omnia::serve(&run_state, servers).await?; // unchanged; resolves after the one-shot
        Ok(exit.get().copied().unwrap_or(ExitStatus::SUCCESS))
    }
}

#[tokio::main]
async fn main() -> std::process::ExitCode {
    use omnia::Parser;
    match omnia::Cli::parse().command {
        // `args` are the guest's argv (everything after `--`); see §5.3.
        omnia::Command::Run { wasm, config, args } => match runtime::run(wasm, config, args).await {
            Ok(status) => status.into(),  // ExitStatus → ExitCode, at the boundary
            Err(error) => { eprintln!("{error:#}"); std::process::ExitCode::FAILURE }
        },
        _ => unreachable!(),
    }
}
```

### 5.3 Argv home

A command's argv flows through the **same** `omnia` CLI surface a server uses — there is no second entry point. Two small pieces, both reused from the floor:

1. **`omnia::Command::Run` grows a trailing `args`** (`crates/omnia/src/lib.rs` lines 66–79): `#[arg(last = true)] args: Vec<String>`, captured after `--`. So a command is launched as `omnia run <wasm> -- greet Ada` (or `omnia --config omnia.toml -- …`). A server passes no `--`, so `args` is empty and its path is byte-identical to today. This deliberately keeps **one** CLI surface — a command shares the `omnia` entrypoint rather than parsing its own argv — and single-surface convergence is worth the `--` separator. (A distributor that wants a bare `mycli greet Ada` UX can ship a thin renamed wrapper; that is packaging, not floor.)
2. **`StoreBase::builder().args(...)`** — landed (`crates/omnia/src/store.rs` lines 39–49). The generated `Context` carries `#[runtime(args)] args: Arc<Vec<String>>`, and the `Runtime` derive's `store()` adds `.args(&self.args)` to the builder chain (`crates/host-macros/src/runtime_derive.rs` lines 100–110). For a server `args` is empty, so `.args(&[])` equals today's omitted call (`build` already defaults argv to empty, `store.rs` lines 36–37, 101) — the server path is unchanged either way.

### 5.4 The macro — one `runtime!`, a backend-optional grammar, a one-shot tail

`runtime!` is **not** renamed and **not** split. Two contained changes let it wire a command:

1. **Backend-optional hosts.** The grammar `Host: Backend` becomes `Host` *optionally* `: Backend` (`crates/host-macros/src/runtime.rs` lines 99–106: peek for `:`). A backend-less host contributes no `Backend::connect`, no `#[wasi(...)]` `StoreCtx` field, and no view — so `runtime!({ main: true, hosts: { WasiCli } })` needs no ZST backend and no no-op view. This is strictly more general than today and is the *only* grammar change; servers with backends are unaffected.
2. **A one-shot `main` tail.** When a one-shot trigger (`WasiCli`) is among the hosts, `expand.rs` emits the command-shaped `runtime::run` + `main` of §5.2 (returns `ExitCode`, threads `args`, reads the exit cell) instead of the server-shaped `main` (`crates/host-macros/src/expand.rs` lines 208–223). The shared 80% — `Context`, `StoreCtx`, host-linking (line 98), backend-connect, registry build, **and the `omnia::serve` call** — is identical. The fork is one branch on the `main` tail, not a second macro, not a second lifecycle helper, and not a rename.

The deployment author writes the same `runtime!` every other example uses:

```rust
// examples/cli/runtime.rs (aligned) — the same macro as every reactor host
cfg_if::cfg_if! {
    if #[cfg(not(target_arch = "wasm32"))] {
        use omnia_wasi_cli::WasiCli;
        omnia::runtime!({ main: true, hosts: { WasiCli } }); // backend-less; one-shot tail
    } else { fn main() {} }
}
```

The bare base-only `StoreCtx` (no `#[wasi(...)]` fields) already works: the `StoreContext` derive tolerates a context with only `#[base]` (the hand-written `CliCtx`, `examples/cli/runtime.rs` lines 29–33, proves it), and the base linker already links `wasi:cli/*`, so **no `Host` view is needed**.

A command that also imports an extension WASI capability (say, one that reads `wasi-keyvalue`) writes `hosts: { WasiCli, WasiKeyValue: KeyValueDefault }`: `WasiKeyValue` is *linked* as a capability and *not* driven as a trigger (it has only the no-op `Server::run` default), exactly as in a server deployment. This is how a command stays a command while consuming host capabilities — no special path.

### 5.5 Co-listing — a guardrail, not a separate macro

A one-shot trigger co-listed with a long-lived one would hang: `try_join_all` waits for the forever-server while the command's exit code sits unread (`crates/omnia/src/runtime.rs` line 109). A separate command-only macro could make that combination unrepresentable (§8); this design instead prevents it with a **guardrail**, equally safe and far cheaper, in two layers:

1. **The command tail drives only the one-shot.** When the macro selects the command tail (because `WasiCli` is present, §5.4) it puts **only** the `WasiCli` server in the `servers` vec. Any co-listed capability host (`WasiKeyValue`, `WasiOtel`, …) is still *linked* but, having only the no-op `Server::run` default, contributes nothing to drive — so a command that reads `wasi-keyvalue` is fine, and there is no future to deadlock on.
2. **Co-listing a long-lived trigger is a compile error.** The macro already recognises hosts by name (it name-maps `WasiHttp` → `omnia_wasi_http` via `wasi_ident`, `crates/host-macros/src/expand.rs` lines 260–268), so it can recognise the built-in *trigger* hosts (`WasiHttp`, `WasiMessaging`, `WasiWebsocket`, and future additions) the same way. Wiring any of them alongside `WasiCli` is rejected at expansion time with a clean message ("a one-shot `WasiCli` trigger cannot be co-listed with the long-lived trigger `WasiHttp`; split them into separate deployments"). This turns layer 1's *silent* non-serving into a *loud* error.

A compile error is exactly as strong as "unrepresentable" for the property that matters (you cannot ship the broken deployment) without forking the macro, the lifecycle, and the mental model to get there. (A third-party trigger host the macro does not know by name still falls under layer 1 — linked, not driven — so it degrades safely to "the command runs and exits"; the day that matters, the recogniser is a list to extend, not a redesign.)

### 5.6 Multi-command routing (future), and the path to Level 2

Because CLI now rides `TriggerRouter` and `Routes`, multi-command routing is a pure extension, fully consistent with the other triggers:

- Add a `cli: CliRoutes` field to `Routes` (`crates/omnia/src/routing.rs` lines 104–145) and a `[[route.cli]]` parser to the manifest, keyed on the leading argv token. `WasiCli::run` then `resolve`s that token instead of taking the sole catch-all. The leading-token rule *is* the floor's existing `FirstArgSelector` (`crates/omnia/src/selector.rs` lines 47–61): "the first argument is the identity" is precisely "the first argv token is the subcommand." `GuestId` stays opaque; the floor learns no consumer vocabulary (Law 2 holds).

**Level 2** (a single uniform handler the floor adapts every event — HTTP request, message, argv — into, so the *guest* never changes when triggers swap) is out of scope. But it is reachable *only* because the trigger machinery is unified here: a uniform "event" interface would be one more `TriggerRouter`-selected handler that `WasiCli`/`WasiHttp`/`WasiMessaging` each adapt into, rather than a reconciliation of two divergent worlds.

## 6. Before → after

| Aspect | Today (shipped) | After (this design) |
| --- | --- | --- |
| Guest target | binary, no `crate-type` | `cdylib` |
| Guest export | `fn main` → `wasi:cli/run` | `wasip3::cli::command::export!` + `impl run::Guest` |
| Guest argv/IO | `std::env`, `println!` | `wasip3::cli::environment`, `stdout`/`stderr` helpers |
| Guest filename | `cli-wasm.wasm` | `cli_wasm.wasm` |
| Host | hand-written `Runtime` + `main` (~50 lines) | `omnia::runtime!({ main: true, hosts: { WasiCli } })` (~4 lines) |
| CLI as a trigger | hand-inlined `CommandPre` probe | `WasiCli: Host + Server`, via `TriggerRouter` (like `WasiHttp`) |
| Command lifecycle | inlined in the example | `omnia::serve` (unchanged), one-shot tail |
| Exit status | `std::process::exit` in the host body | `ExitStatus` via a cell, `process::exit` at the `main` boundary |
| Reactor host macro | `omnia::runtime!` | `omnia::runtime!` (**unchanged name**; backend-optional grammar) |
| Floor changes | none | `ExitStatus`, optional `cli` route table, `crates/wasi-cli` host, macro grammar + one-shot tail |

## 7. Implementation plan

The two axes are independent in code; each step compiles and is independently reviewable.

**Floor first (additive, zero risk to servers):**
1. Add the `ExitStatus` newtype to `crates/omnia/src/runtime.rs`; re-export from `crates/omnia/src/lib.rs`. (`StoreBase::builder().args` is already landed.)
2. Add an empty `cli: CliRoutes` table to `Routes` and a `routes().cli()` accessor (`crates/omnia/src/routing.rs`); the manifest `[[route.cli]]` parser is deferred to §5.6 (the table stays empty → catch-all until then).

**The trigger host:**
3. Add `crates/wasi-cli` (host-only) with `WasiCli: Host + Server` per §5.1.
4. Optionally rewrite the hand-written `examples/cli/runtime.rs` to construct `WasiCli` directly (cell + `serve`) — validates the host before any macro work and slims it to ~20 lines.

**Host macro (no rename):**
5. Make the `runtime!` grammar backend-optional (`crates/host-macros/src/runtime.rs`); ensure backend-less hosts emit no `StoreCtx`/view/connect.
6. Add the always-emitted `#[runtime(args)] args` field + the derive's `.args(&self.args)` (`crates/host-macros/src/runtime_derive.rs`), and grow `omnia::Command::Run` a trailing `args` (`crates/omnia/src/lib.rs`).
7. Emit the one-shot `main` tail + co-list guardrail when a `WasiCli` host is present (`crates/host-macros/src/expand.rs`).

**Guest + example:**
8. Rewrite `examples/cli/guest.rs` as a `cdylib` reactor (`wasip3::cli::command::export!` + `async impl exports::cli::run::Guest`); add `crate-type = ["cdylib"]` to the `cli-wasm` entry in `examples/Cargo.toml`.
9. Rewrite `examples/cli/runtime.rs` as `omnia::runtime!({ main: true, hosts: { WasiCli } })`; update `examples/cli/README.md` for the `cli_wasm.wasm` filename and the `omnia run <wasm> -- …` shape.

## 8. Alternatives considered and rejected

- **A `server!`/`cli!` macro split, with a `runtime!`→`server!` rename and a command-only `omnia::run` lifecycle parallel to `serve`.** This would erase the surface divergence (example shape) but introduce a deeper one: a "command world" with its own macro, its own lifecycle helper, and a hand-rolled `CommandPre` probe duplicating `TriggerRouter`. It would violate Level-1 convergence — moving a guest from a CLI trigger to an HTTP trigger would become a *path* change (cross from `cli!`/`omnia::run` to `server!`/`omnia::serve`), not a *wiring* change. Its apparent wins dissolve once a command is seen as a one-shot trigger: exit codes ride a side channel like every trigger's response (§5.2), the co-list footgun is a compile-time guardrail (§5.5), the no-op linker is *correct* (a trigger drives an export; §5.1), the backend requirement is one grammar relaxation (§5.4), and the "vestigial" background tasks are cheap-or-useful (§3). It would also strand §5.6's multi-command routing, which wants the very `TriggerRouter`/`FirstArgSelector` such a split bypasses. **Rejected.**
- **One macro with a mode switch** (e.g. `runtime!({ command: WasiCli, … })`). Unnecessary: `WasiCli` is just a host in the `hosts` list, and "command vs. server" is read from whether a one-shot trigger is present — no new grammar axis. The co-list guardrail (§5.5) covers the one invalid combination. **Rejected.**
- **Keep `wasi:cli` invocation in a hand-written host** (today's `examples/cli/runtime.rs`). Correct but bespoke; it re-implements `TriggerRouter` and skips the instantiation metrics every real trigger records. **Rejected** in favour of the `WasiCli` trigger.
- **Keep the binary guest.** The binary is the more idiomatic *standalone* command (§4.3), but the repo optimizes for one consistent guest mental model; a lone binary guest is exactly the divergence a newcomer trips on. **Rejected** in favour of the `cdylib`.

## 9. Risks and invariants

- **`process::exit` fidelity.** The exit code is set on the cell by `WasiCli::run` and applied by `main` *after* `serve` returns, so destructors on the `serve` path run normally; only the final, intended `process::exit` at the boundary skips unwinding (as a one-shot's terminal step should). Guest stdout/stderr is flushed by `call_run`'s completion (the guest's `blocking_write_and_flush`) before the cell is read.
- **Co-list guardrail must be enforced.** §5.5 is the one place a wrong wiring could hang. The guardrail is a compile-time rejection in `expand.rs`; landing it in the same step as the one-shot tail (plan step 7) keeps the invariant and its enforcement together.
- **Single CLI surface changes `Command::Run`.** Growing `Run` a trailing `args` is additive and gated behind `--`, so existing `omnia run <wasm>` and `omnia run --config …` invocations are unchanged; only `-- <guest args>` is new.
- **Instance-per-call unchanged.** The command is instantiated fresh on a new store and discarded; `build_store` guards (epoch deadline, fuel, memory limiter) still apply, now via `Runtime::instantiate` rather than the hand-written host.
- **Metrics gap closed.** Routing the command through `Runtime::instantiate` records `instantiation_duration_us` / `pool_instantiation_errors` for commands too — the hand-written host skipped these.
- **Law 2 preserved.** `wasi:cli` stays a standard WASI interface and `GuestId` stays opaque; the future `[[route.cli]]` table (§5.6) keys on an argv token without the floor learning any consumer vocabulary.

## 10. References

- [`wasip3::cli::command::export!`](https://docs.rs/wasip3/latest/wasip3/cli/command/macro.export.html) — guest-side export macro for `wasi:cli/command` ([`wasi-rs`](https://github.com/bytecodealliance/wasi-rs)); the §4 export-side parallel to `wasip3::http::service::export!`.
- [`wasmtime_wasi::p2::bindings::Command`](https://docs.rs/wasmtime-wasi/latest/wasmtime_wasi/p2/bindings/struct.Command.html) — host-side command invoke (`CommandPre`, `Command`, `call_run`, `I32Exit`) used by `WasiCli::run`; distinct from the guest SDK above.
- `crates/wasi-http/src/host.rs`, `crates/wasi-http/src/host/server.rs` — the `Host` + `Server` + `TriggerRouter` trigger pattern §5.1 mirrors (including out-of-band response delivery).
- `crates/wasi-messaging/src/host/server.rs` — a second `TriggerRouter`-driven trigger, showing the probe-then-resolve shape is the floor's norm.
- `examples/http/guest.rs` — the reactor export-side idiom §4 mirrors.
- `crates/omnia/src/runtime.rs` — `omnia::serve` (the unchanged lifecycle helper) and the home for the new `ExitStatus`.
- `crates/omnia/src/routing.rs`, `crates/omnia/src/selector.rs` — `TriggerRouter`, `Routes`, and the `FirstArgSelector` that the future `[[route.cli]]` table (§5.6) reuses.
- `crates/omnia/src/create.rs` — the base linker that links `wasi:cli/*` (p2 + p3), which is why `WasiCli::add_to_linker` is a correct no-op.
- `crates/host-macros/src/{lib.rs,runtime.rs,expand.rs,runtime_derive.rs,store_context.rs}` — the macro machinery the backend-optional grammar, the argv field, and the one-shot `main` tail extend.
- `crates/omnia/src/{store.rs,traits.rs}` — `StoreBase::builder` and the `Runtime`/`Server`/`Host` traits the trigger reuses unchanged.
