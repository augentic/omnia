# Omnia Documentation

Omnia is a lightweight, secure runtime for WebAssembly (WASI) components, built on [wasmtime](https://github.com/bytecodealliance/wasmtime). Application code compiles to a sandboxed WebAssembly **guest**; a native **host** runtime provides it with services — HTTP, storage, messaging, SQL, model completions, and more — through standard WASI interfaces. The same guest runs unchanged against in-memory defaults on a laptop or production services (Redis, Kafka, Azure, PostgreSQL) in deployment.

This documentation is organised so you can start small and go deeper as you need to:

## Start here

- **[Getting Started](getting-started.md)** — build and run your first guest in about ten minutes. No external services required.

## How-to guides

### Core workflow

| Guide | What it covers |
| ----- | -------------- |
| [Writing Guests](guides/writing-guests.md) | HTTP handlers, WASI capabilities, tracing, command-mode guests |
| [Composing a Runtime](guides/composing-a-runtime.md) | The `runtime!` macro, choosing hosts and backends, server vs command mode |
| [Multi-Guest Deployments](guides/multi-guest-deployments.md) | The `omnia.toml` manifest, HTTP routing across guests, mounts, guest-to-guest linking |
| [Testing Guests and Runtimes](guides/testing.md) | Seam tests with `omnia-testkit`, the integration-first policy, live tests |

### Capabilities in depth

| Guide | What it covers |
| ----- | -------------- |
| [SQL and the Guest ORM](guides/sql-and-orm.md) | Raw SQL, the `entity!` macro, query builders, joins |
| [Document Store](guides/document-store.md) | JSON documents, the filter language, sorting and pagination |
| [Messaging](guides/messaging.md) | Pub/sub, request-reply, fan-out, message handlers, WebSockets |
| [Model Completions and MCP](guides/model-completions.md) | The `omnia:model` interface, grants and host-injected tools, the `genai` and `cursor` backends, serving MCP tools from a guest |

### Operating

| Guide | What it covers |
| ----- | -------------- |
| [Production Backends](guides/production-backends.md) | Swapping in-memory defaults for Redis, Kafka, PostgreSQL, Azure, and others from the [`backends`](https://github.com/augentic/backends) repo |
| [Deploying Omnia](guides/deployment.md) | Release builds, ahead-of-time compilation, container images, backing services, readiness |
| [Performance Tuning](guides/performance-tuning.md) | The instance-per-call cost model, pooling knobs, and measuring with the bench harness |

## Explanation

- **[Architecture](Architecture.md)** — how the runtime is put together: the three-layer design, the guest registry, execution flow, instance pooling, and host-mediated dispatch.
- **[Security Model](security-model.md)** — what the sandbox guarantees, how capabilities are granted, and what the runtime does not protect against.
- **[Glossary](glossary.md)** — shared terminology (**runtime core**, **host-injected tools**, **Law 2**, and friends).

## Reference

- **[WASI Interfaces](reference/wasi-interfaces.md)** — every interface crate, its default backend, and which production backends implement it.
- **[Model Interface](reference/model.md)** — `omnia:model/completion` types, validation rules, and the replay fixture format.
- **[CLI](reference/cli.md)** — the `run` and `compile` subcommands and their flags.
- **[Configuration](reference/configuration.md)** — runtime environment variables and the deployment manifest format.

## When something goes wrong

- **[Troubleshooting](troubleshooting.md)** — common build, startup, and runtime failures with fixes.

## Working examples

Every feature described here has a runnable example under [`examples/`](../examples/README.md). Each example pairs a guest (`guest.rs`, compiled to `.wasm`) with a host runtime (`runtime.rs`, a native binary), and its README gives exact build and run commands.
