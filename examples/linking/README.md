# Host-mediated dynamic linking

Proves **Phase 2** of [`rfcs/guest-registry.md`](../../rfcs/guest-registry.md):
one guest reaches another through an interface the *host* satisfies at runtime,
carried over in-process [wRPC](https://github.com/bytecodealliance/wrpc).

## What it shows

- `responder` ([`responder.rs`](responder.rs)) **exports** `omnia:link/echo`. It
  declares no trigger of its own, so it is reachable *only* via dispatch.
- `router` ([`router.rs`](router.rs)) **imports** `omnia:link/echo` and exposes
  `run(message)`. Its component does not satisfy the import.
- [`omnia.toml`](omnia.toml) names `omnia:link/echo` in the router's `link`
  allow-list. The floor polyfills that import onto the shared linker and, at
  startup, wires the serve side of every linked interface.

When `router.run("hello")` calls the imported `echo("responder", "hello")`:

```mermaid
flowchart LR
  router["router.run<br/>(imports echo)"] -->|"echo(\"responder\", \"hello\")"| sel["FirstArgSelector<br/>target = responder"]
  sel --> guard["reject resources (§4.5)<br/>depth bound (§6.6)"]
  guard -->|"in-process wRPC"| resp["responder.echo<br/>(fresh instance per call)"]
  resp -->|"\"responder echoes: hello\""| router
```

The selector reads the leading argument (`"responder"`) to pick the target and
forwards it through; the responder is instantiated **fresh per call**
(instance-per-call) and discarded.

The floor stays generic (Law 2): `link` and the selector operate on the opaque
interface string `omnia:link/echo` and opaque guest ids — Omnia never parses the
interface's meaning.

## Build the guests

A whole-workspace `wasm32-wasip2` build fails on the native-only host crates, so
build the two guest components explicitly:

```bash
cargo build -p examples \
  --example linking-responder-wasm \
  --example linking-router-wasm \
  --target wasm32-wasip2
```

This emits `target/wasm32-wasip2/debug/examples/linking_responder_wasm.wasm` and
`linking_router_wasm.wasm` (the underscored names the manifest points at).

## Run

```bash
cargo run --example linking -- run --config examples/linking/omnia.toml
```

The host starts, polyfills the router's import, and wires the responder's serve
side. Because the router exports a plain `run` (not an HTTP/messaging trigger),
the end-to-end dispatch is exercised by the integration test rather than inbound
traffic:

```bash
# after building the guests above (do NOT `cargo clean` in between):
cargo nextest run -p omnia --test linking
```

The test builds the registry from this manifest, calls `router.run`, and asserts
it returns the responder's echo — and that the responder is instantiated exactly
once per dispatched call.
