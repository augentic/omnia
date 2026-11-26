# Deploying Omnia

This guide covers taking a host runtime from `cargo run` to production: release builds, ahead-of-time compilation, container images, backing services, and readiness detection.

## What you ship

An Omnia deployment is two artifacts:

1. **The host binary** — your `runtime!` crate compiled for the target platform. It embeds the wasmtime engine and the backends you declared.
2. **The guest(s)** — `.wasm` components (or pre-compiled `.bin` files), plus an `omnia.toml` manifest for multi-guest deployments.

The two are versioned independently: a guest update ships a new `.wasm` without rebuilding the host, and a backend swap rebuilds the host without touching guests.

## Release builds

Build guests and the host in release mode:

```bash
cargo build --release --target wasm32-wasip3 -p <guest-crate>
cargo build --release -p <host-crate>
```

The workspace release profile already enables LTO, `opt-level = "s"`, and symbol stripping, which keeps the host binary small.

## Ahead-of-time compilation

By default the host JIT-compiles `.wasm` at startup. For faster cold starts, pre-compile guests to serialized wasmtime components:

```bash
<runtime> compile guest.wasm -o guest.bin
<runtime> run guest.bin
```

Three constraints:

- Compile-affecting options (`MAX_FUEL`, `MEMORY_RESERVATION`, `MEMORY_GUARD_SIZE`, `BRANCH_HINTING`, `DEBUG_SYMBOLS`, `GENERATE_ADDRESS_MAP`) must be identical at compile time and run time. The compiler takes them explicitly as a `CompileOptions` value rather than reading its own environment — `CompileOptions::default()` is the runtime's environment defaults; `RuntimeOptions::load_env()?.compile_options()` is the current environment's — so a `build.rs` that compiles a guest is not steered by whatever the build shell exports. A deployment given its options in code, through `DeploymentBuilder::options`, loads only an artifact compiled under their `compile_options()`. The artifact is for the host's target unless the compile names another (`--target <triple>` on the CLI, the `target` argument in code); a guest compiled into a binary is compiled for the binary's target.
- The generated `runtime!` `main` runs `compile` under the compile-affecting settings its own environment sets, so compile with the production values set (see [CLI reference](../reference/cli.md#compile-jit-feature)). A host built with `--no-default-features` (dropping `jit`) refuses `compile` and can *only* load pre-compiled `.bin` files — useful for minimizing the production binary.
- A `.bin` is native code and a **trusted operator input**: the runtime loads whichever format a declared guest turns out to be, and the settings check above is a compatibility check, not an authenticity check — see the [security model](../security-model.md).

## Container images

The recommended pattern is a multi-stage build that produces a small Alpine image running the host as a non-root user, with the guest supplied at run time. For your own host crate:

```dockerfile
FROM rust:alpine AS build
RUN apk add --no-cache build-base cmake perl
WORKDIR /app
COPY . .
RUN cargo build --release --bin my-runtime

FROM alpine:latest
COPY --from=build /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/ca-certificates.crt
COPY --from=build /app/target/release/my-runtime /bin/server
USER 10001:10001
EXPOSE 8080
ENTRYPOINT ["/bin/server", "run"]
CMD ["/app.wasm"]
```

Key points of the pattern:

- **Guest as a volume, not a layer.** The `ENTRYPOINT` is the host's `run` subcommand; `CMD` names the guest path. Mount the `.wasm` (or a manifest plus guests) at run time, so guest updates don't rebuild the image:

```bash
docker run -v ./guest.wasm:/app.wasm -p 8080:8080 my-runtime
```

- **Non-root user** and CA certificates copied in for TLS-speaking backends.

## Backing services for local testing

Everything in this repository runs against in-memory defaults, so no external services are needed here. Production backends (Redis, Kafka, NATS, PostgreSQL, and so on) are the concern of the [`omnia-backends`](https://github.com/augentic/omnia-backends) repository, which is where service definitions for exercising them locally belong — see [Production Backends](production-backends.md#verifying-against-the-real-service) for how its live tests verify a backend against the real service.

## Configuration and secrets

Everything is environment variables — backend connection strings, runtime limits, pool sizes (see [Configuration](../reference/configuration.md)). In containers, inject them through your orchestrator's secret mechanism; nothing is read from files except the deployment manifest (`OMNIA_MANIFEST`), the `registries` configuration it names, and the releases its package store holds.

Two variables deserve attention in production:

- `RUST_LOG` — a server defaults to `warn`; set at least `info` (or start the binary with `-v`) so startup, readiness, and trigger-server logs are emitted.
- `OTEL_EXPORTER_OTLP_ENDPOINT` — point at your collector; the host exports traces and metrics with no further wiring. The runtime's own metrics — the `pool_*` occupancy gauges, the concurrency gate's `invocations_*` gauges, counter, and histogram, `message_counter`, `event_counter`, and `processing_errors` — export whatever `RUST_LOG` is, as does every metric a backend or a guest records, including through a `tracing` event's `monotonic_counter.`, `counter.`, `histogram.`, or `gauge.` field. Unset (and no per-signal `OTEL_EXPORTER_OTLP_*_ENDPOINT`), nothing is exported — guest telemetry included.

## Readiness and health

After bootstrap completes — hosts linked, backends connected, guests pre-instantiated, trigger servers wired — the runtime logs one line at `info`:

```text
omnia ready
```

with the mode and guest count attached. Orchestrators (or a log-watching startup probe) should key on this line; a process that is up but hasn't logged it is still connecting backends or compiling guests. For HTTP deployments, the listening port only accepts traffic after this point, so a TCP readiness probe on `HTTP_ADDR` is equivalent.

An HTTP deployment can also answer a probe itself. Set [`HTTP_HEALTH_PATH`](../reference/configuration.md#http-trigger) (for example `/healthz`), and the HTTP trigger answers a request for exactly that path with `200`. The path:

- enters no guest and takes no slot of the [concurrency gate](../reference/configuration.md#concurrency-gate), so it stays `200` while HTTP answers `503` at capacity;
- serves a liveness probe, so a busy pod is not restarted, or a readiness probe, so a busy pod stays in rotation rather than pushing its load onto peers that are just as full;
- needs no `Host`, since it is answered before the request target is checked;
- wins over a guest route that covers it, while a path below it goes to the guest;
- exists only where the HTTP trigger runs, and a deployment with no guest serving HTTP logs a warning when the variable is set.

Not ready is a refused connection. On shutdown the trigger drops its listener before in-flight requests drain, so a probe made after that point is refused rather than answered.

## Shutdown

A server stops on `SIGTERM` or `SIGINT`. The first signal logs one line at `info`:

```text
waiting for in-flight tasks to exit; send another signal to quit immediately
```

Each trigger then takes no new work and waits for the work it already holds:

- HTTP accepts no more connections and drops its listener. Each open connection finishes the request it is serving, then closes.
- Messaging takes no more messages from its transport, and each delivery it has taken runs to completion, its reply included. A message it has not taken is left to the transport. A durable one such as Kafka redelivers it after the restart, and the in-memory broker and NATS core lose it.
- WebSocket takes no more events from its backend, and each event it has taken runs to completion.

The process exits 0 once every trigger has drained. A second signal quits at once, abandoning whatever is still in flight, and also exits 0.

How long the drain takes depends on what is in flight at the stop:

- An invocation holding a slot of the [concurrency gate](../reference/configuration.md#concurrency-gate) ends within [`GUEST_TIMEOUT_MS`](../reference/configuration.md#guest-limits). Its default of 30 s is also Kubernetes' default grace period.
- A message delivery taken before the stop but still waiting for a slot takes one as others end, so a saturated gate drains for longer.
- An HTTP response body is bounded only by [`HTTP_STREAM_TIMEOUT_MS`](../reference/configuration.md#http-trigger), which is unset by default, so a streaming response holds the drain open until it ends.

An orchestrator whose grace period ends first kills the process mid-drain, cutting off whatever is still in flight.

## Production checklist

- [ ] Release build; consider AOT (`compile`) plus a `jit`-less host for fastest, smallest deployments
- [ ] Backend env vars set and validated (the host fails at startup if a backend cannot connect)
- [ ] `GUEST_TIMEOUT_MS`, `MAX_MEMORY_BYTES` sized for your workload ([tuning guide](performance-tuning.md))
- [ ] `RUST_LOG=info` and `OTEL_EXPORTER_OTLP_ENDPOINT` set
- [ ] Readiness keyed on the `omnia ready` log line, TCP on `HTTP_ADDR`, or an HTTP probe on `HTTP_HEALTH_PATH`
- [ ] Orchestrator grace period (`terminationGracePeriodSeconds` on Kubernetes) longer than `GUEST_TIMEOUT_MS`, plus `HTTP_STREAM_TIMEOUT_MS` where responses stream ([shutdown](#shutdown))
- [ ] Mounts limited to the directories guests actually need, read-only unless writes are required ([security model](../security-model.md))
- [ ] `[env] inherit` (or `--env`) names each host variable guests read, since they see no other ([configuration](../reference/configuration.md#deployment-manifest-omniatoml))
- [ ] `CONFIG_KEYS`, or the keys the runtime compiles in, names each host variable guests read through `omnia:config`, since `ConfigDefault` serves no other ([configuration](../reference/configuration.md#default-backends))
