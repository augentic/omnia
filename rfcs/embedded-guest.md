# Design: Embedded Guest — Single-Binary Runtime + Guest

> Note: the `DeploymentBuilder` baseline described here (path-based `config`/`wasm` resolution in `build()`) predates the manifest-first builder, which now takes an `omnia::Manifest` value; the integration points in §5/§9 would target `Manifest`/`omnia::main` instead.

> Status: Design proposal — lets `runtime!` optionally embed a guest component, pre-compiled to a cwasm artifact at build time, so a deployment ships as one self-contained native binary (runtime core + guest) with no filesystem dependency and no JIT in the shipped artifact. Complements — does not replace — the `omnia run <wasm>` / `--config` deployment paths. Depends: the `runtime!` macro, `Source` guest acquisition, `omnia compile` (`crates/omnia/src/options/compile.rs`). Relates: [backend-selection](backend-selection.md) (the opposite trade: one binary, *dynamic* composition; this design is one binary, *fully static* composition).

## 1. Motivation

Today a deployment is always two artifacts: the host binary and the guest `.wasm` (or pre-compiled `.bin`), joined at startup by a path — `omnia run <wasm>` or a manifest `source`. For the common "one runtime, one guest, ship it" case this forces the operator to distribute, version, and co-locate two files, and either:

- ship the pre-compiled artifact and keep its compile-affecting settings in lockstep with the binary out-of-band, or
- ship raw wasm and compile the `jit` feature (Cranelift) into the production binary just to load it.

The goal: a crate author writes

```rust
omnia::runtime!({
    guest: "guest.cwasm",   // produced into OUT_DIR by build.rs
    hosts: {
        WasiHttp: HttpDefault,
        WasiKeyValue: KeyValueDefault,
    }
});
```

and `cargo build` emits a single binary that runs its guest with plain `./mybinary` — no path, no `jit` feature, no runtime codegen (the compiler ran at build time; the shipped binary only deserializes).

## 2. Current state: what exists, what is missing

The two halves of the pipeline already exist:

- **Producing cwasm** — `compile()` (`crates/omnia/src/options/compile.rs`) is the whole compiler: `Engine::new(&Config::from(&RuntimeOptions::load()?))`, `Component::from_file`, `Component::serialize`.
- **Loading cwasm** — `load_component()` (`crates/omnia/src/deployment/source.rs`) already classifies the artifact by content (wasmtime-serialized ELF vs raw wasm) and deserializes the former; admission of pre-compiled artifacts is gated by the `DeploymentBuilder::precompiled()` typestate's unsafe `build`, since wasmtime's settings-compatibility check is not an authenticity check. An embedded artifact is trusted by construction (baked into the binary at build time), so the embedded path makes that attestation internally.

What is missing is the middle:

1. `Source` only takes a `PathBuf`; there is no bytes-backed source for `include_bytes!` data.
2. `DeploymentBuilder` resolves only `config` / `wasm` paths; the generated `main` (`omnia::main` in `crates/omnia/src/runtime.rs`) requires the `run` subcommand with a guest argument.
3. Nothing produces the cwasm *during* the host crate's build.
4. `runtime!` has no `guest:` option.

## 3. The constraint that shapes the design

cwasm compilation cannot live in the `runtime!` proc macro:

- A pre-compiled artifact is rejected at load unless the deserializing engine matches the **exact wasmtime version** and the **compile-affecting settings** it was built with (`MAX_FUEL`, `BRANCH_HINTING`, `MEMORY_RESERVATION`, `MEMORY_GUARD_SIZE` — see the safety comment in `source.rs`). `host-macros` has no wasmtime dependency, and giving it one would version-couple the proc macro to the runtime and bloat macro compilation.
- The guest is a separate `wasm32-wasip2` artifact that typically does not exist yet when the host crate's macros expand (the examples' two-step build).
- `include_bytes!` in the macro *expansion* gets cargo dependency tracking for free; heavy file I/O inside macro expansion gets none.

Therefore the split is: **a build script produces `$OUT_DIR/guest.cwasm`; the macro `include_bytes!`s it and threads the bytes into the deployment.** Build scripts run before macro expansion, so the ordering is guaranteed by cargo.

## 4. Proposed design

### 4.1 Bytes-backed source (`crates/omnia/src/deployment/source.rs`)

`Source` gains an embedded kind alongside the path kind:

```rust
pub enum SourceKind {
    Path(PathBuf),
    Embedded(&'static [u8]),
}
```

`load` on the embedded kind calls `unsafe { Component::deserialize(engine, bytes) }` with the same safety rationale and error context as `deserialize_file` today, and the same `#[cfg(feature = "jit")]` fallback (`Component::new`) so embedding *raw* wasm also works under `jit`. This is the "OCI would land as another kind" seam the module doc already anticipates.

### 4.2 Builder and entry point (`crates/omnia/src/deployment.rs`, `crates/omnia/src/runtime.rs`)

- `DeploymentBuilder::embedded(name, bytes)` records an embedded guest.
- `build()` resolution order becomes: explicit `config` (or `OMNIA_CONFIG`) → CLI `wasm` path → embedded bytes → error. The embedded guest is the *default*, still overridable from the command line for development.
- `omnia::main` takes the embedded guest as an `Option`:

```rust
pub struct EmbeddedGuest {
    pub name: &'static str,
    pub bytes: &'static [u8],
}

pub async fn main<B, H>(mode: Mode, embedded: Option<EmbeddedGuest>) -> ExitCode;
```

- CLI ergonomics: when a guest is embedded, invoking the binary with **no subcommand** behaves as `run` (so `./mybinary` and `./mybinary run` are equivalent, and `--mount` / `--link` / `-- args` still apply). Without an embedded guest, behaviour is unchanged.

### 4.3 Build-time precompile — `omnia-build` (new crate)

A small library crate for `[build-dependencies]`, wrapping the three lines of `compile()`:

```rust
// build.rs of the host binary crate
fn main() -> anyhow::Result<()> {
    omnia_build::embed_guest("../target/wasm32-wasip2/release/guest.wasm")
}
```

`embed_guest`:

1. builds the engine from `Config::from(&RuntimeOptions)` — identical to `compile()`,
2. sets `Config::target(env::var("TARGET")?)` so the cwasm targets the *binary's* triple, not the build host (build scripts run on the host; without this, cross-compiled binaries embed unloadable cwasm),
3. `Component::from_file` → `serialize` → writes `$OUT_DIR/guest.cwasm`,
4. emits `cargo:rerun-if-changed=<guest path>`,
5. exports the compile-affecting option values as `cargo:rustc-env=OMNIA_EMBED_*` (see §5).

`omnia-build` depends on omnia with the `jit` feature (it needs the compiler); the host binary's *runtime* omnia dependency can drop `jit` entirely — Cranelift runs during the build and stays out of the shipped artifact.

Producing the guest `.wasm` itself stays outside `omnia-build` v1: the documented pattern is the existing two-phase build (`cargo build --target wasm32-wasip2` first). A convenience that shells out to cargo from `build.rs` needs a separate `--target-dir` to avoid the target-directory lock and is an open question (§8).

### 4.4 Macro surface (`crates/host-macros/src/runtime/{parse,codegen}.rs`)

One new option, `guest:`, a string-literal filename resolved against `OUT_DIR`:

```rust
omnia::runtime!({
    guest: "guest.cwasm",
    hosts: { ... }
});
```

Codegen adds to the generated module:

```rust
const GUEST: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/guest.cwasm"));
```

and the generated `main` passes `Some(omnia::EmbeddedGuest { name: env!("CARGO_PKG_NAME"), bytes: GUEST })` — `None` when `guest:` is absent, so every existing `runtime!` user is untouched. The `include_bytes!` sits in the *user's* crate after expansion, so `OUT_DIR` and rebuild tracking resolve against the user's crate, not `host-macros`.

## 5. Compile/run settings drift

`RuntimeOptions::load()` reads the environment. For the embedded path the compile happens on the build machine and the load on the deployment machine; if the compile-affecting variables differ, `deserialize` rejects the artifact at startup. The artifact must be self-consistent **by construction**, not by operator discipline:

- `omnia-build` bakes the four compile-affecting values into the binary via `cargo:rustc-env` (`OMNIA_EMBED_MAX_FUEL`, …).
- When constructing the engine for a deployment whose guest is embedded, those baked values **override** the runtime environment for the compile-affecting settings (non-compile-affecting options stay env-driven). A runtime env var that disagrees logs a warning naming the baked value.

This keeps the invariant: an embedded binary always loads its own guest.

## 6. What ships, and the raw-wasm variant

- **omnia**: `SourceKind::Embedded`, builder/entry-point plumbing, the `guest:` macro option.
- **omnia-build**: the build-dependency crate.
- Docs: `docs/guides/composing-a-runtime.md` (the embedded pattern), `docs/reference/cli.md` (no-subcommand default).

A cheaper variant falls out for free: `guest:` pointing at a raw `.wasm` (embedded via the same `include_bytes!`) with the `jit` feature enabled — no `build.rs`, no settings-drift concern, at the cost of Cranelift in the binary and JIT at startup. The macro surface is identical either way, so cwasm embedding is purely a build-side upgrade.

## 7. Acceptance criteria

1. A crate using `runtime!({ guest: ..., hosts: ... })` plus an `omnia-build` build script produces one binary; `./mybinary` runs the guest with no arguments, no adjacent files, and no `jit` feature compiled in.
2. `./mybinary run <other.wasm>` and `--config` still override the embedded guest; a `runtime!` without `guest:` is byte-for-byte unaffected.
3. Deserialization of the embedded guest cannot fail from environment drift: compile-affecting settings are baked at build time (§5).
4. Cross-compiling the host binary embeds cwasm for the *target* triple.
5. A seam test drives an embedded-guest runtime through a real WASI boundary (per the testing policy, guest–host seam, not unit mocks).
6. `cargo make lint` and `cargo make ci` stay green; `omnia-build` joins the workspace.

## 8. Open questions

1. Nested guest build: should `omnia-build` optionally invoke `cargo build --target wasm32-wasip2` (separate `--target-dir`) itself, or stay artifact-in / artifact-out with the two-phase build documented?
2. Multi-guest embedding: `guest:` as a list (several `include_bytes!`, identities from filenames) — or does multi-guest stay manifest territory?
3. Should the embedded binary retain the `run` / `compile` subcommands at all, or become argument-transparent (everything after the binary name is guest argv, matching `mode: command` ergonomics)?
4. Where the baked compile-affecting settings live: `cargo:rustc-env` consumed by `RuntimeOptions`, or a small const struct passed through `EmbeddedGuest`?
5. Compression: cwasm artifacts are large (often several MB); is a `zstd`-compressed embed worth the startup decompression cost, given `initialize_copy_on_write_image` wants the raw bytes anyway?

## 9. References

- `crates/omnia/src/options/compile.rs` — the compile pipeline `omnia-build` wraps.
- `crates/omnia/src/deployment/source.rs` — `Source` / `load_component`, the loading seam gaining the embedded kind.
- `crates/omnia/src/deployment.rs` — `DeploymentBuilder` resolution.
- `crates/omnia/src/runtime.rs` — `omnia::main`, the generated entry point.
- `crates/host-macros/src/runtime/{parse,codegen}.rs` — the macro this design extends.
- [backend-selection](backend-selection.md) — the complementary dynamic-composition design.
