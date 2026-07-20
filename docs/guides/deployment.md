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
cargo build --release --target wasm32-wasip2 -p <guest-crate>
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

- Compile-affecting options (`MAX_FUEL`, `MEMORY_RESERVATION`, `MEMORY_GUARD_SIZE`, `BRANCH_HINTING`) must be identical at compile time and run time.
- The generated `runtime!` `main` only handles `run`; expose `compile` from a custom `main` (see [CLI reference](../reference/cli.md#compile-jit-feature)). A host built with `--no-default-features` (dropping `jit`) can *only* load pre-compiled `.bin` files — useful for minimizing the production binary.
- A `.bin` is native code and a **trusted operator input**: the CLI loads it on that basis, while the programmatic API requires an explicit `unsafe` attestation (`DeploymentBuilder::precompiled()` or `GuestArtifact::precompiled`). The settings check above is a compatibility check, not an authenticity check — see the [security model](../security-model.md).

## Container images

The repository's [`Dockerfile`](../../Dockerfile) shows the pattern: a multi-stage build that produces a small Alpine image running the host as a non-root user, with the guest supplied at run time. Adapted for your own host crate:

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
- The examples use the same pattern via [`docker/service.yaml`](../../docker/service.yaml): build context at the repo root, the built guest mounted to `/${COMPOSE_PROJECT_NAME}.wasm`, and `OTEL_GRPC_URL` pointed at a collector container.

> Note: the in-repo `Dockerfile` predates the workspace reorganisation — it references a root `src/` directory and `--bin` targets that no longer exist, so treat it as the pattern to copy into your own host crate rather than something to build directly from this repo.

## Backing services for local testing

The [`docker/`](../../docker/) directory ships a compose file per service so production backends can be exercised locally:

| File                   | Service                   | Matches backend                         |
| ---------------------- | ------------------------- | --------------------------------------- |
| `docker/redis.yaml`    | Redis                     | `omnia-redis`                           |
| `docker/kafka.yaml`    | Apache Kafka              | `omnia-kafka`                           |
| `docker/nats.yaml`     | NATS / JetStream          | `omnia-nats`                            |
| `docker/postgres.yaml` | PostgreSQL (+ `init.sql`) | `omnia-postgres`                        |
| `docker/mongodb.yaml`  | MongoDB                   | `omnia-mongodb`                         |
| `docker/otelcol.yaml`  | OpenTelemetry Collector   | `omnia-opentelemetry` / `OTEL_GRPC_URL` |

```bash
docker compose -f docker/redis.yaml up -d
REDIS_URL=redis://localhost:6379 cargo run -p my-runtime -- run guest.wasm
```

These are also the services the backends repo's [live tests](production-backends.md#verifying-against-the-real-service) run against.

## Configuration and secrets

Everything is environment variables — backend connection strings, runtime limits, pool sizes (see [Configuration](../reference/configuration.md)). In containers, inject them through your orchestrator's secret mechanism; nothing is read from files except the deployment manifest (`OMNIA_CONFIG`).

Two variables deserve attention in production:

- `RUST_LOG` — set at least `info` so startup, readiness, and trigger-server logs are emitted.
- `OTEL_GRPC_URL` — point at your collector; the host exports traces and metrics (including pool-occupancy gauges) with no further wiring.

## Readiness and health

After bootstrap completes — hosts linked, backends connected, guests pre-instantiated, trigger servers wired — the runtime logs one line at `info`:

```text
omnia ready
```

with the mode and guest count attached. Orchestrators (or a log-watching startup probe) should key on this line; a process that is up but hasn't logged it is still connecting backends or compiling guests. For HTTP deployments, the listening port only accepts traffic after this point, so a TCP readiness probe on `HTTP_ADDR` is equivalent.

## Production checklist

- [ ] Release build; consider AOT (`compile`) plus a `jit`-less host for fastest, smallest deployments
- [ ] Backend env vars set and validated (the host fails at startup if a backend cannot connect)
- [ ] `GUEST_TIMEOUT_MS`, `MAX_MEMORY_BYTES` sized for your workload ([tuning guide](performance-tuning.md))
- [ ] `RUST_LOG=info` and `OTEL_GRPC_URL` set
- [ ] Readiness keyed on the `omnia ready` log line (or TCP on `HTTP_ADDR`)
- [ ] Mounts limited to the directories guests actually need, read-only unless writes are required ([security model](../security-model.md))
