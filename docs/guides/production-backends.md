# Production Backends

The in-tree `*Default` backends make development friction-free, but production deployments usually need real infrastructure. The [`backends`](https://github.com/augentic/backends) repository provides drop-in implementations of the same WASI interfaces against real services. Swapping one in changes a single line in your host runtime — guests are untouched.

## What's available

| Backend crate | Service | WASI interface(s) |
| ------------- | ------- | ----------------- |
| `omnia-redis` | Redis | keyvalue |
| `omnia-nats` | NATS / JetStream | keyvalue, messaging, blobstore |
| `omnia-kafka` | Apache Kafka (optional Schema Registry) | messaging |
| `omnia-postgres` | PostgreSQL | sql |
| `omnia-mongodb` | MongoDB | blobstore |
| `omnia-azure-blob` | Azure Blob Storage | blobstore |
| `omnia-azure-table` | Azure Table Storage | docstore |
| `omnia-azure-vault` | Azure Key Vault | vault |
| `omnia-azure-id` | Azure Managed Identity | identity |
| `omnia-opentelemetry` | OpenTelemetry Collector (OTLP gRPC) | otel |
| `omnia-genai` | LLM provider APIs (OpenAI, Anthropic, Gemini, ...) | model |
| `omnia-cursor` | `cursor-agent` CLI | model |

The model backends (`genai`, `cursor`) are covered in [Model Completions and MCP](model-completions.md).

## Swapping a backend

A backend is any type that implements `omnia::Backend` (connection management) plus the `WasiXxxCtx` context trait for its interface. In the `runtime!` macro, replace the default with the production client:

```rust
use omnia_redis::Client as Redis;
use omnia_wasi_http::{HttpDefault, WasiHttp};
use omnia_wasi_keyvalue::WasiKeyValue;
use omnia_wasi_otel::{OtelDefault, WasiOtel};

omnia::runtime!({
    hosts: {
        WasiHttp: HttpDefault,
        WasiOtel: OtelDefault,
        WasiKeyValue: Redis,        // was: KeyValueDefault
    }
});
```

Add the backend crate to your host's `Cargo.toml`:

```toml
[dependencies]
omnia-redis = "0.28"
```

For local development against unreleased omnia changes, the `backends` workspace patches all `omnia`/`omnia-wasi-*` crates to a sibling checkout via `[patch.crates-io]` — keep both repositories checked out side by side and mirror that pattern if your host lives in a third workspace.

## Configuration

Every backend reads its connection settings from environment variables at startup (the `FromEnv` pattern). The most common ones:

| Backend | Key variables |
| ------- | ------------- |
| Redis | `REDIS_URL` (default `redis://localhost:6379`), `REDIS_MAX_RETRIES` |
| NATS | `NATS_ADDR`, `NATS_TOPICS`, `NATS_JWT`/`NATS_SEED` |
| Kafka | `KAFKA_BROKERS`, `COMPONENT`, `KAFKA_TOPICS`, `KAFKA_CONSUMER_GROUP`, `KAFKA_USERNAME`/`KAFKA_PASSWORD`, `KAFKA_REGISTRY_URL` |
| PostgreSQL | `POSTGRES_URL`, `POSTGRES_POOL_SIZE`; named pools via `POSTGRES_POOLS` + `POSTGRES_URL__<NAME>` |
| MongoDB | `MONGODB_URL` (must include a default database) |
| Azure Blob | `AZURE_BLOB_ENDPOINT`; service-principal via `AZURE_TENANT_ID`/`AZURE_CLIENT_ID`/`AZURE_CLIENT_SECRET`, else `az login` |
| Azure Table | `AZURE_STORAGE_ACCOUNT`, `AZURE_STORAGE_KEY`, optional `AZURE_TABLE_ENDPOINT` (Azurite) |
| Azure Key Vault | `AZURE_KEYVAULT_URL` + Azure credentials |
| OpenTelemetry | `OTEL_GRPC_URL` (default `http://localhost:4317`) |

Each crate's README in the `backends` repository documents its complete variable set.

## Verifying against the real service

Backend crates ship `#[ignore]`-gated **live tests** (`tests/live.rs`) that drive the backend's `WasiXxxCtx` implementation against the real service. They never run in CI; run them locally with the service up and credentials set:

```bash
# example: Redis
export REDIS_URL=redis://localhost:6379
cargo nextest run -p omnia-redis --run-ignored all
```

This is the recommended smoke test after wiring a new backend into a deployment.
