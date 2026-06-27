# Design: Aligning `wasi:cli` — a `cdylib` Guest and the `server!` / `cli!` Macro Split

> Status: Accepted — a committed design, not a study. `examples/cli` diverges from every other example on **two** axes: its guest is a binary (not a `cdylib` reactor) and its host is hand-written (not macro-wired). This document commits to erasing both: the guest becomes a `cdylib` exporting `wasi:cli/run` via `wasip3`, and the host gains a first-class command form. Rather than bend the server-shaped `runtime!` around a command, we **split the host macro**: `runtime!` is renamed to `server!` (the long-lived-server form) and a corollary `cli!` runs a single command to completion through a new floor helper, `omnia::run`. The rename is a hard break — no compatibility alias.
>
> Owns: the committed design for aligning `examples/cli` on both axes. Touches: `examples/cli/{guest.rs,runtime.rs,README.md}`, `examples/Cargo.toml`, `crates/omnia/src/{runtime.rs,lib.rs}`, `crates/host-macros/src/{lib.rs,expand.rs,runtime.rs,runtime_derive.rs}`, every reactor example's `runtime.rs` (the `runtime!`→`server!` rename), and the crate READMEs/`docs` that name `runtime!`. Depends: the landed `StoreBase::builder` argv setter, the `Runtime` / `StoreContext` derives, `omnia::serve`, the base linker's `wasi:cli` linking (`crates/omnia/src/create.rs`), `wasmtime-wasi`'s p2 `Command` / `CommandPre` bindings, and [`wasip3`](https://docs.rs/wasip3) guest export macros (Bytecode Alliance [`wasi-rs`](https://github.com/bytecodealliance/wasi-rs)).

## 1. Abstract

`examples/cli` is the repo's only *command* (a guest that exports `wasi:cli/run`, invoked once); every other example is a *reactor* (a `cdylib` exporting a handler that a long-lived `Server` drives on each inbound event). To keep the first command self-contained it took the two localized shortcuts — a binary guest and a hand-written host — which leave it reading nothing like the deployments a newcomer learns first.

This document commits to removing both divergences:

- **[§4 Guest](#4-guest-alignment--cdylib-reactor)** — rewrite the guest as a `cdylib` reactor exporting `wasi:cli/run` via `wasip3::cli::command::export!` (the same guest-SDK layer HTTP uses), so it shares the target shape, filename rule, and cfg convention of every other guest.
- **[§5 Host](#5-host-alignment--the-server--cli-split)** — give commands a first-class host form *without* pretending a command is a server. Rename `runtime!` → `server!` (the long-lived-server macro, behaviour unchanged) and add a corollary `cli!` that runs one command to completion via a new floor helper `omnia::run` (the command analog of `omnia::serve`).

The unifying idea: **every guest is a `cdylib` exporting a handler; every host is either `server!` (drives long-lived servers) or `cli!` (runs one command).** A command's handler is `wasi:cli/run`; its driver is `omnia::run`. The command/server distinction becomes a first-class, teachable axis rather than a hand-written special case — and the deadlocking co-list of a command with a server (the central hazard of the trigger-host alternative, §8) becomes *unrepresentable* instead of merely documented.

This explicitly **rejects** modelling `wasi:cli` as a trigger host inside `server!` (the approach the rest of this document's earlier drafts explored); that path is retained as a rejected alternative in [§8](#8-alternatives-considered-and-rejected).

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
| Lifecycle | instantiate, `call_run`, map exit code, exit (`runtime.rs` lines 96–105) | `omnia::serve` joins long-lived servers forever |
| argv | injected via `StoreBase::builder().args(&args)` (`runtime.rs` lines 49–54) | no path exists; the generated `store()` omits `.args` |
| Exit status | a guest `i32` honoured via `std::process::exit` | none — servers do not return a status |

Both halves are *correct*; they are just bespoke. A reader who has learned Omnia as a server cannot place this file, because nothing else in the repo looks like it.

## 3. The command / server dichotomy

A *command* and a *server* differ on five axes the current floor conflates into one server-shaped path:

| Axis | Server (`runtime!`) | Command |
| --- | --- | --- |
| Lifecycle | `try_join_all` over long-lived servers (`crates/omnia/src/runtime.rs` line 109) | one invoke, then exit |
| Exit status | none (`Server::run -> Result<()>`) | an `i32` (`I32Exit` / `Ok(Err(()))`) |
| argv | none | consumes argv |
| Backend / view | one per host, **required** by the parser (`crates/host-macros/src/runtime.rs` lines 99–106) | none (a bare `wasi:cli` command) |
| Background tasks | epoch interruption + pool sampling justified | vestigial for a one-shot |

The design follows from this table: a command is not a server, so it gets its own corollary entry point (`cli!` + `omnia::run`) that shares the *linking* machinery with `server!` but not the *lifecycle*. Forcing a command through `server!` instead — the trigger-host alternative — produces a vestigial artifact on every one of these axes and an unsafe co-list footgun ([§8](#8-alternatives-considered-and-rejected)).

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

## 5. Host alignment — the `server!` / `cli!` split

> Give commands a first-class host form. Rename `runtime!` → `server!`, add a corollary `cli!`, and back both with a shared expansion plus two floor lifecycle helpers: the existing `omnia::serve` and a new `omnia::run`.

### 5.1 `omnia::run` — the command runner

Today the invoke logic is hand-inlined in the example (`examples/cli/runtime.rs` lines 96–105). Lift it into `crates/omnia/src/runtime.rs`, beside `serve`, but **return** the status instead of exiting from inside:

```rust
/// Run the single `wasi:cli/run` exporter once and report its exit status.
///
/// The command analog of [`serve`]: `serve` joins long-lived servers; this runs
/// one command to completion. Routes through `Runtime::instantiate` so a command
/// records the same `instantiation_duration_us` / `pool_instantiation_errors`
/// metrics a trigger does (which the hand-written host skips). Unlike `serve`,
/// it spawns no epoch/pool background tasks — a command runs to completion.
pub async fn run<R: Runtime>(runtime: &R) -> Result<ExitStatus>
where
    R::StoreCtx: WasiView + WrpcView + 'static,
{
    // Probe: a guest is command-capable iff `CommandPre::new(..)` is Ok.
    let guest = runtime
        .registry()
        .guests()
        .find(|g| CommandPre::new(g.instance_pre().clone()).is_ok())
        .context("no guest exports wasi:cli/run")?;

    let mut store = runtime.build_store(runtime.store());
    let instance = runtime.instantiate(guest.instance_pre(), &mut store).await?;
    let command = Command::new(&mut store, &instance)?;

    Ok(match command.wasi_cli_run().call_run(&mut store).await {
        Ok(Ok(())) => ExitStatus::SUCCESS,
        Ok(Err(())) => ExitStatus::from(1),
        // A guest `std::process::exit` (or panic) surfaces as an `I32Exit`
        // error rather than `Ok(Err(()))`; honour its code.
        Err(error) => match error.downcast_ref::<I32Exit>() {
            Some(exit) => ExitStatus::from(exit.0),
            None => return Err(error),
        },
    })
}
```

`ExitStatus` is a thin floor newtype over the guest's `i32`, exported from `omnia`:

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

The process exit happens at the boundary (the generated `main`), never inside a `Server`. This resolves the trigger-host design's "exit codes can't ride `Result<()>`" problem ([§8](#8-alternatives-considered-and-rejected)) by construction: a command is not a `Server`.

This addition is purely additive — it does not touch `serve` — so it carries no risk to any existing deployment, and it slims the hand-written host immediately even before `cli!` exists.

### 5.2 `server!` and `cli!` — one expansion, two tails

Both macros parse the same grammar (`{ main: bool, hosts: { Host: Backend, … } }`) and emit the same `Context` + `StoreCtx` + host-linking + backend-connect (`crates/host-macros/src/expand.rs`). They differ only in the lifecycle tail and argv:

| | `server!` (was `runtime!`) | `cli!` |
| --- | --- | --- |
| `hosts` | required (a server with none does nothing useful) | optional (a bare command has none) |
| Lifecycle tail | build `servers` vec → `omnia::serve` (`expand.rs` lines 76–78) | call `omnia::run` |
| argv | none | `Context` carries `args`; `store()` adds `.args(&self.args)` |
| `main` | `omnia::Cli::parse()` → `Command::Run { wasm, config }`, returns `Result<()>` | parses `<wasm> [args…]` directly, returns `ExitCode` |

Because a command is single-guest by nature, `cli!` accepts the *optional* `hosts: { … : Backend }` grammar too — for a command that imports extension WASI (say, one that reads `wasi-keyvalue`). Those hosts are **linked** (the `Host` / connect / view half of `expand.rs`) but **not driven as servers**: the generated `run` calls `omnia::run`, not `omnia::serve`. So the two macros share ~80% of the expansion and the split is clean rather than duplicative — one parameterized `Expanded` with two public entry points in `crates/host-macros/src/lib.rs`.

The deployment author writes, for the bare command:

```rust
// examples/cli/runtime.rs (with cli!) — parity with the ~12-line server form
cfg_if::cfg_if! {
    if #[cfg(not(target_arch = "wasm32"))] {
        omnia::cli!({ main: true });   // no hosts → no backend, no view
    } else { fn main() {} }
}
```

…and `cli!` generates the command-shaped `main` (mirroring what the example hand-rolls today, lines 62–106):

```rust
#[tokio::main]
async fn main() -> std::process::ExitCode {
    // A command owns its argv: `<binary> <wasm> [guest args…]`.
    let mut argv = std::env::args().skip(1);
    let Some(wasm) = argv.next() else {
        eprintln!("usage: <wasm> [args…]");
        return std::process::ExitCode::FAILURE;
    };
    let mut args = vec!["cli".to_string()];
    args.extend(argv);

    match runtime::run(PathBuf::from(wasm), args).await {
        Ok(status) => status.into(),            // ExitStatus → ExitCode, at the boundary
        Err(error) => { eprintln!("{error:#}"); std::process::ExitCode::FAILURE }
    }
}
```

The generated `runtime::run` builds the registry and `Context { registry, args }`, then calls `omnia::run(&context)` — the cli counterpart to the `server!` module's `runtime::run` that calls `omnia::serve`.

The bare base-only `StoreCtx` (no `#[wasi(...)]` fields) already works: the `StoreContext` derive tolerates a context with only `#[base]` (the hand-written `CliCtx`, `examples/cli/runtime.rs` lines 29–33, proves it), and the base linker already links `wasi:cli/*`, so **no `Host` impl is needed**.

### 5.3 Argv home

The argv path needs only two pieces, both small:

1. **`StoreBase::builder().args(...)`** — the type-state builder's optional `.args` setter. **Landed.** It is the WASI-policy home that ends the re-inlining the hand-written host used to do.
2. **An opt-in `#[runtime(args)]` field attribute on the `Runtime` derive** (`crates/host-macros/src/runtime_derive.rs`). When present, `store()` adds `.args(&self.<field>)` to the builder chain (`crates/host-macros/src/runtime_derive.rs` lines 100–110). `cli!` emits `#[runtime(args)] args: Arc<Vec<String>>` on its `Context`; `server!` never emits it, so the server path is unchanged.

Crucially, the `cli!` `main` parses `<wasm> [args…]` itself and does **not** route through `omnia::Cli` / `Command::Run`. So the shared CLI surface (`crates/omnia/src/lib.rs` lines 66–91) stays untouched — a command *is* its own CLI and should not be forced under an `omnia run` subcommand. This avoids growing `Command::Run` an `args` field and threading it through the generated `main`, which the trigger-host alternative would have required.

### 5.4 The rename: `runtime!` → `server!`, no alias

The `runtime` proc-macro (`crates/host-macros/src/lib.rs` lines 26–33) is renamed to `server`; `crates/omnia/src/lib.rs` re-exports `server` (and the new `cli`) in place of `runtime`. This is a **hard break with no compatibility alias**: a stale `runtime!` is a clean compile error pointing at the new name, not a silent deprecation.

Blast radius (all updated in the same change): every reactor example's `runtime.rs` (`examples/*/runtime.rs`), and every crate README / `docs` file that names `runtime!` (the `wasi-*` crate READMEs, `crates/omnia/README.md`, `crates/host-macros/README.md`, the top-level `README.md`, and `docs/Architecture.md`). The `Runtime` *derive* and its `#[runtime(...)]` helper attribute keep their names — they are a separate macro (`#[proc_macro_derive(Runtime, attributes(runtime))]`) and the `Runtime` trait it implements is unchanged.

Downstream consumers outside this repo (e.g. the `backends` workspace) must update their `runtime!` call sites; that is the intended, visible cost of the clean break.

### 5.5 Lifecycle and exit semantics — resolved

The `server!`/`cli!` split dissolves every lifecycle wrinkle the trigger-host approach had to work around:

- **Exit codes leave the boundary clean.** `omnia::run` returns `ExitStatus`; the generated `main` converts to `ExitCode`. No `std::process::exit` inside a `Server`, and no need to widen `Server::run`'s return type (which would touch every host).
- **No vestigial background tasks.** `omnia::run` does not spawn `drive_epoch` / `sample_pool` — those exist for long-lived servers. A command runs to completion. (If a future command needs a wall-clock deadline, epoch driving can be added to `omnia::run` then, gated on a configured timeout.)
- **Co-listing is unrepresentable.** You cannot express "a command plus an HTTP server" and get a hung join, because `cli!` runs exactly one command and never calls `serve`. The invariant "a command is its own deployment" is enforced by the shape of the macro, not by documentation. (Contrast `serve`'s `try_join_all`, `crates/omnia/src/runtime.rs` line 109: a successful command co-listed with a long-lived server under one `serve` would never let the join complete.)
- **No vestigial host machinery.** No no-op `Host`, no ZST backend, no empty view, no server-only-host parser feature — `cli!`'s `hosts` is simply optional.

### 5.6 Multi-command routing (future)

`cli!` is single-guest today: `omnia::run` probes for the sole `wasi:cli/run` exporter. If commands ever multiply within one deployment, the floor already has the parts — `TriggerRouter::build` for the capability probe (`crates/omnia/src/routing.rs` lines 248–264) and a `FirstArgSelector`-style read of the leading argv token (`crates/omnia/src/selector.rs`) — so `omnia::run` can grow a `[[route.cli]]`-keyed selection without changing the `cli!` surface. Out of scope here; noted so the door stays open. Law 2 holds: such a table keys on an argv token without the floor learning any consumer vocabulary, and `GuestId` stays opaque.

## 6. Before → after

| Aspect | Today (shipped) | After (this design) |
| --- | --- | --- |
| Guest target | binary, no `crate-type` | `cdylib` |
| Guest export | `fn main` → `wasi:cli/run` | `wasip3::cli::command::export!` + `impl run::Guest` |
| Guest argv/IO | `std::env`, `println!` | `wasip3::cli::environment`, `stdout`/`stderr` helpers |
| Guest filename | `cli-wasm.wasm` | `cli_wasm.wasm` |
| Host | hand-written `Runtime` + `main` (~50 lines) | `omnia::cli!({ main: true })` (~6 lines) |
| Command lifecycle | inlined in the example | `omnia::run` (floor, beside `serve`) |
| Exit status | `std::process::exit` in the host body | `ExitStatus` returned to `main` |
| Reactor host macro | `omnia::runtime!` | `omnia::server!` (hard rename) |
| Floor changes | none | `omnia::run` + `ExitStatus`, `cli!`, the rename, opt-in `#[runtime(args)]` |

## 7. Implementation plan

The two axes are independent in code; this is the recommended order (each step compiles and is independently reviewable).

**Floor first (additive, zero risk to servers):**
1. Add `omnia::run` + `ExitStatus` to `crates/omnia/src/runtime.rs`; re-export both from `crates/omnia/src/lib.rs`. (`StoreBase::builder().args` is already landed.)
2. Optionally rewrite `examples/cli/runtime.rs`'s hand-written host to call `omnia::run` now — it slims to ~20 lines and validates the helper before any macro work.

**Host macro:**
3. Add the opt-in `#[runtime(args)]` field attribute to the `Runtime` derive (`crates/host-macros/src/runtime_derive.rs`).
4. Factor `crates/host-macros/src/expand.rs` into a shared expansion with two tails; add the `cli` proc-macro entry point (`crates/host-macros/src/lib.rs`) and re-export `omnia::cli`.

**Rename (hard break):**
5. Rename the `runtime` proc-macro to `server` (`crates/host-macros/src/lib.rs`), re-export `omnia::server`, and update every `runtime!` call site and doc (`examples/*/runtime.rs`, crate READMEs, `docs/Architecture.md`, top-level `README.md`). No alias.

**Guest + example:**
6. Rewrite `examples/cli/guest.rs` as a `cdylib` reactor (`wasip3::cli::command::export!` + `async impl exports::cli::run::Guest`), dispatching over `wasip3::cli::environment` and stdout/stderr helpers; add `crate-type = ["cdylib"]` to the `cli-wasm` entry in `examples/Cargo.toml`.
7. Rewrite `examples/cli/runtime.rs` as `omnia::cli!({ main: true })`; update `examples/cli/README.md` for the `cli_wasm.wasm` filename and the new command shape.

## 8. Alternatives considered and rejected

- **`wasi:cli` as a trigger host inside `server!`** (the earlier-draft approach). A `WasiCli` host with a no-op `add_to_linker` (the base linker already links `wasi:cli/*`, so linking again errors — `crates/omnia/src/create.rs` lines 159–160), a capability-probing `Server`, and — because the parser requires a backend per host (`crates/host-macros/src/runtime.rs` lines 99–106) — either a ZST "noop backend" the command never reads or a new server-only-host parser feature, plus an empty view. Exit codes can't ride `Server::run -> Result<()>`, forcing `std::process::exit` *inside* a trigger; the epoch/pool background tasks are vestigial for a one-shot; and — worst — co-listing `WasiCli` with `WasiHttp` deadlocks the command behind the HTTP accept loop because `serve`'s `try_join_all` never completes (`crates/omnia/src/runtime.rs` line 109), a footgun the macro *invites* with no compile-time guard. The `cli!` split makes all of this unnecessary and the co-list state unrepresentable. **Rejected.**
- **One macro with a mode switch** (e.g. `runtime!({ command: WasiCli, … })`). Keeps server and command entangled in one code path; the invalid "command + server" combination stays representable and must be hand-rejected at expansion time. The two-macro split moves that rejection into the grammar. **Rejected.**
- **Keep `runtime!` as a deprecated alias for `server!`.** Dilutes the rename's pedagogical value (the whole point is to make "server vs. command" legible) and leaves a second name to maintain. Per the committed scope, the rename is a clean break. **Rejected.**
- **Keep the binary guest.** The binary is the more idiomatic *standalone* command (§4.3), but the repo optimizes for one consistent guest mental model; a lone binary guest is exactly the divergence a newcomer trips on. **Rejected** in favour of the `cdylib`.

## 9. Risks and invariants

- **Rename churn.** §5.4 touches every example and many docs, plus downstream consumers. The clean break is intentional, but it is the one non-additive change; landing it as its own step (plan step 5) keeps the diff legible.
- **Idiom regression (guest).** §4 removes the canonical std command form from the repo. Accepted (§4.3) for guest-shape uniformity; an `omnia-wasi-cli` guest helper can restore ergonomics later without reintroducing the binary divergence.
- **Instance-per-call unchanged.** The command is still instantiated fresh on a new store and discarded; `build_store` guards (epoch deadline, fuel, memory limiter) still apply, now via `omnia::run` rather than the hand-written host.
- **Metrics gap closed.** Routing the command through `Runtime::instantiate` records `instantiation_duration_us` / `pool_instantiation_errors` for commands too — the hand-written host skipped these.
- **Law 2 preserved.** `wasi:cli` stays a standard WASI interface and `GuestId` stays opaque; the future `[[route.cli]]` table (§5.6) keys on an argv token without the floor learning any consumer vocabulary.

## 10. References

- [`wasip3::cli::command::export!`](https://docs.rs/wasip3/latest/wasip3/cli/command/macro.export.html) — guest-side export macro for `wasi:cli/command` ([`wasi-rs`](https://github.com/bytecodealliance/wasi-rs)); the §4 export-side parallel to `wasip3::http::service::export!`.
- [`wasmtime_wasi::p2::bindings::Command`](https://docs.rs/wasmtime-wasi/latest/wasmtime_wasi/p2/bindings/struct.Command.html) — host-side command invoke (`CommandPre`, `Command`, `call_run`, `I32Exit`) used by `omnia::run`; distinct from the guest SDK above.
- `examples/http/guest.rs` — the reactor export-side idiom §4 mirrors.
- `examples/config/guest.rs`, `examples/keyvalue/guest.rs` — reactor guests combining `wasip3` exports with `omnia-wasi-*` import bindings.
- `crates/omnia/src/runtime.rs` — `omnia::serve` (the server lifecycle helper) and the home for the new `omnia::run`.
- `crates/omnia/src/create.rs` — the base linker that links `wasi:cli/*` (p2 + p3), which is why a command needs no `Host`.
- `crates/host-macros/src/{lib.rs,expand.rs,runtime.rs,runtime_derive.rs,store_context.rs}` — the macro machinery the `server!`/`cli!` split and the `#[runtime(args)]` attribute extend.
- `crates/omnia/src/{store.rs,traits.rs,routing.rs,selector.rs}` — `StoreBase::builder`, the `Runtime`/`Server` traits, and the routing/selector parts a future multi-command `omnia::run` would reuse.
- `crates/wasi-otel/src/host.rs` (line 57) — the no-op `Server` precedent, retained only as context for the rejected trigger-host alternative (§8).
