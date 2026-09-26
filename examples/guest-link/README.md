# Two guests talking to each other

If you have written microservices, you already know this shape: service A calls service B. In Omnia, A and B are WebAssembly **guests** (`.wasm` components) running inside one native **host** process. They do not open sockets to each other. The host copies the arguments across, runs B, and copies the result back.

Think of the host as an in-process service mesh. This example is the smallest version of that: a caller (`router`) and a callee (`responder`) that share one function, `echo`.

## The two guests

| Guest                       | Role                                                           | Like…                                           |
| --------------------------- | -------------------------------------------------------------- | ----------------------------------------------- |
| [`responder`](responder.rs) | **Exports** `echo`. Has no HTTP (or other) port of its own.    | An internal service with no public listener     |
| [`router`](router.rs)       | **Imports** `echo`, and exposes `run(message)` which calls it. | An API that needs to call that internal service |

`router.wasm` does not contain an implementation of `echo`. If the host did not wire the import, the guest would not even start.

The shared contract is [`wit/link.wit`](wit/link.wit). WIT is the IDL of the WebAssembly component model — the analogue of a protobuf service or an OpenAPI spec:

```wit
interface echo {
    echo: func(target: string, message: string) -> string;
}
```

The first argument is not a domain field. It is the **guest name** of who should answer — `"responder"` in the default path, the same string as `name` in the manifest. The host reads that string to pick a target, then forwards every argument through unchanged, so the callee sees the same signature.

## What happens on a call

When something invokes `router.run("hello")`, the router calls `echo("responder", "hello")`:

```mermaid
flowchart LR
  router["router.run"] -->|"echo(\"responder\", \"hello\")"| host["host picks target<br/>from the first argument"]
  host --> resp["new responder instance<br/>runs echo"]
  resp -->|"\"responder echoes: hello\""| router
```

Three things surprise people coming from HTTP services:

1. **Same process.** There is no network hop. The host copies the arguments and the result in memory.
2. **A new instance every time.** The responder is created for this call and thrown away afterwards, like a Lambda invocation. Nothing in its memory survives for the next call. Put durable state in key-value or SQL.
3. **Only copyable data.** Strings, numbers, records, lists — fine. File handles, streams, and other live resources cannot cross, because they belong to the caller's instance, not the callee's.

The host does not parse what `echo` means, and nothing declares it. `example:link/echo` lies outside the runtime's own `wasi:` and `omnia:` namespaces, so the host relays it: any guest importing it reaches whichever guest exports it, and an import nothing exports fails at start.

## Quick start

The deployment is embedded in [`runtime.rs`](runtime.rs): the examples package's `build.rs` compiles both guests for `wasm32-wasip2`, and the `runtime!` invocation embeds them, naming them `responder` and `router` — the names the router dispatches by — rather than by their files' stems. So a bare `run` works from any directory with no build step first:

```bash
export RUST_LOG=info
cargo run --example guest-link -- run
```

If startup succeeds, the import was wired: `router` can resolve `echo`, and `responder` is registered to serve it. The process then sits in server mode. To actually *see* a round-trip printed, use the register variant below.

The same deployment as a TOML file is [`omnia.toml`](omnia.toml), which reads each guest from disk at its first use rather than at start (so under this server-mode host, which never calls the router, it boots and sits without loading either); those build first:

```bash
cargo build -p examples \
  --example guest-link-responder-wasm \
  --example guest-link-router-wasm \
  --target wasm32-wasip2

cargo run --example guest-link -- run --manifest examples/guest-link/omnia.toml
```

Cargo writes underscored names: `target/wasm32-wasip2/debug/examples/guest_link_responder_wasm.wasm` and `guest_link_router_wasm.wasm`. The manifest points at those paths.

## The same deployment, built in Rust

[`dynamic.rs`](dynamic.rs) does not compile the guest list into the binary. It builds an `omnia::Manifest` at runtime, reading the guests built above into bytes — so, like the embedded pair, they load at boot and startup proves the wiring — and hands it to the generated host:

```rust
let manifest = Manifest::new()
    .guest(GuestEntry::new("responder", std::fs::read(responder_wasm)?))
    .guest(GuestEntry::new("router", std::fs::read(router_wasm)?));

host::run(DeploymentBuilder::new().manifest(manifest))?;
```

Handing `GuestEntry::new` the *path* instead declares a guest that loads at its first use, as a `[[guest]]` in a manifest file does.

```bash
cargo run --example guest-link-dynamic
```

## Adding a guest after startup

[`extra.rs`](extra.rs) also exports `echo`, but it is not in the manifest. [`register.rs`](register.rs) starts the two-guest deployment (naming its guests by path, so each loads at its first use), then loads `extra` with `Runtime::register` and asks the router to call it — resolving the router through `Runtime::guest`, which is what loads it:

```bash
cargo build -p examples \
  --example guest-link-responder-wasm \
  --example guest-link-router-wasm \
  --example guest-link-extra-wasm \
  --target wasm32-wasip2

cargo run --example guest-link-register
```

You should see two lines: one reply tagged `from extra`, and one from `responder`. `Runtime::deregister` then removes `extra`. A call that is already in flight is allowed to finish — it holds its own instance.

```rust
let bytes = std::fs::read(extra_wasm)?;
runtime.register("extra", bytes).await?;
```

The bytes may be a raw `.wasm`, compiled at registration, or `omnia compile` output, deserialized as native code — the runtime tells them apart the way wasmtime does. Verifying them (digest, signature, provenance) is the install pipeline's job, before they reach the runtime; the registry records their digest either way.

## Also in this directory

- **`echo-slow` / `run-slow`.** Same call, but the responder waits 5ms on a host timer before answering. This shows the path still works when the callee is actually waiting, not just returning immediately.
- **[`relay.rs`](relay.rs).** Exports `echo` *and* imports it again. Each call decrements a hop count and dispatches onward, so one inbound call becomes a chain. Used to exercise the depth ceiling (`MAX_DISPATCH_DEPTH`, default 8).

The broader picture — manifests, HTTP routing across guests, mounts — is in [Multi-Guest Deployments](../../docs/guides/multi-guest-deployments.md).
