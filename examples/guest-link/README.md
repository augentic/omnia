# Guest link (host-mediated dynamic linking)

Proves host-mediated dynamic linking: one guest reaches another through an interface the *host* satisfies at runtime, carried over in-process [wRPC](https://github.com/bytecodealliance/wrpc).

## What it shows

- `responder` ([`responder.rs`](responder.rs)) **exports** `omnia:link/echo`. It declares no trigger of its own, so it is reachable *only* via dispatch.
- `router` ([`router.rs`](router.rs)) **imports** `omnia:link/echo` and exposes `run(message)`. Its component does not satisfy the import.
- [`omnia.toml`](omnia.toml) names `omnia:link/echo` in the router's `link` allow-list. The runtime core polyfills that import onto the shared linker and, at startup, wires the serve side of every linked interface.

When `router.run("hello")` calls the imported `echo("responder", "hello")`:

```mermaid
flowchart LR
  router["router.run<br/>(imports echo)"] -->|"echo(\"responder\", \"hello\")"| sel["FirstArgSelector<br/>target = responder"]
  sel --> guard["reject resources (§4.5)<br/>depth bound (§6.6)"]
  guard -->|"in-process wRPC"| resp["responder.echo<br/>(fresh instance per call)"]
  resp -->|"\"responder echoes: hello\""| router
```

The selector reads the leading argument (`"responder"`) to pick the target and forwards it through; the responder is instantiated **fresh per call** (instance-per-call) and discarded.

The interface also carries an async-typed dual, `echo-slow: async func`, called by the router's async-lifted `run-slow`. It rides the same dispatch, registered with `func_new_concurrent` instead of `func_new_async` (an async-typed import fails the sync registration's typecheck), and the responder parks on a `wasi:clocks` timer before answering — proving the round-trip against a callee that is genuinely pending.

The runtime core stays generic (Law 2): `link` and the selector operate on the opaque interface string `omnia:link/echo` and opaque guest ids — Omnia never parses the interface's meaning.

## Quick Start

This example deploys two guests from a manifest, so build and run stay manual:

```bash
# build the guests
cargo build -p examples \
  --example guest-link-responder-wasm \
  --example guest-link-router-wasm \
  --target wasm32-wasip2

# run the host — the manifest path is compiled in (runtime! `config:`),
# so a bare `run` works from any directory
export RUST_LOG=info,opentelemetry_sdk=off
cargo run --example guest-link -- run

# or with an explicit manifest
cargo run --example guest-link -- run --config examples/guest-link/omnia.toml
```

This emits `target/wasm32-wasip2/debug/examples/guest_link_responder_wasm.wasm` and `guest_link_router_wasm.wasm` (the underscored names the manifest points at).

## Integration test

```bash
# after building the guests above (do NOT `cargo clean` in between):
cargo nextest run -p omnia --test guest_link
```

The test builds the registry from this manifest, calls `router.run` and `router.run-slow`, and asserts each returns the responder's echo — and that the responder is instantiated exactly once per dispatched call.
