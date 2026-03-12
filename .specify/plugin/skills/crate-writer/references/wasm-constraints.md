# Runtime Constraint Translations for WASM

This document defines how to translate `[runtime]` constraints from Specify artifacts into Omnia SDK patterns for WASM guest components. The artifacts describe source behaviors factually; this document prescribes the WASM/Omnia translation.

## Constraint Translation Table

| Specify `[runtime]` Constraint | Omnia Translation | Required Traits | Pattern Reference |
| --- | --- | --- | --- |
| Source uses in-memory cache with startup loading | On-demand **cache-aside**: `StateStore` for caching + original data source trait for fetching. Data loaded on first request (or cache miss), not at startup. | `StateStore` + data source trait (`TableStore` or `HttpRequest`) | [statestore.md](../examples/capabilities/statestore.md) |
| Source uses `setTimeout`/`setInterval` for periodic refresh | **TTL-based cache expiry** via `StateStore`. Set TTL when writing; stale entries auto-evicted and re-fetched on next request. | `StateStore` | [statestore.md](../examples/capabilities/statestore.md) |
| Source uses circuit breaker library | **Provider-only HTTP**. The Omnia runtime handles transport-level resilience. No circuit breaker crate needed. | `HttpRequest` | [guardrails.md](guardrails.md) |
| Source caches OAuth tokens in process memory | **`Identity` provider**. Token acquisition and caching delegated to the Omnia runtime via `Identity::access_token`. | `Identity` | [capabilities.md](capabilities.md#identity) |
| Source uses global logger singleton | **`tracing`** crate. Use `tracing::info!`, `tracing::debug!`, etc. No global logger construction. | (none — `tracing` is a dependency, not a provider trait) | [guardrails.md](guardrails.md) |
| Source uses APM library (e.g., New Relic) | **OTEL instrumentation**. Annotate handlers with `#[omnia_wasi_otel::instrument]`. Use `tracing::info!(monotonic_counter.X = 1)` for metrics. | (none — OTEL is build-time, not a provider trait) | [guest-patterns.md](guest-patterns.md) |

## Detailed Translations

### In-Memory Startup Cache → Cache-Aside

When the artifacts say the source loads data from a data store on application start and holds it in process memory:

1. WASM guests are stateless — no long-lived process memory across requests
2. Translate to **on-demand cache-aside**:
   - On each request, check `StateStore` for cached data
   - On cache miss, fetch from the original data source (use `TableStore` for databases/managed table stores, `HttpRequest` for external APIs)
   - Write the fetched data to `StateStore` with a TTL
3. Do NOT assume a separate cron/ETL component pre-populates the cache — the handler itself must fetch from the data source on cache miss
4. The handler's provider bounds must include BOTH `StateStore` AND the data source trait

### Periodic Refresh → TTL Cache

When the artifacts say the source uses `setTimeout`/`setInterval` for periodic cache refresh:

1. WASM guests cannot run background timers
2. Translate to TTL-based cache: set a TTL when writing to `StateStore`
3. Stale entries are automatically evicted; re-fetched on next cache miss
4. The TTL duration should approximate the source's refresh interval

### Circuit Breaker → Provider HTTP

When the artifacts say the source uses a circuit breaker library (e.g., `opossum`, `cockatiel`):

1. WASM guests use provider-only HTTP via `HttpRequest::fetch`
2. The Omnia runtime manages transport-level concerns
3. No circuit breaker crate needed — remove the pattern entirely

### OAuth Token Caching → Identity Provider

When the artifacts say the source acquires and caches OAuth tokens:

1. Use `Identity::access_token(provider, identity_name)` for token acquisition
2. The identity name comes from `Config::get(provider, "AZURE_IDENTITY")` (or equivalent config key)
3. Token caching and refresh are handled by the Omnia runtime
4. Handler bounds need `Config + Identity + HttpRequest` (Config for the identity name, Identity for the token, HttpRequest for the authenticated call)

### Global Logger → Tracing

When the artifacts say the source uses a global logger:

1. Replace with `tracing` macros: `tracing::info!`, `tracing::debug!`, `tracing::warn!`, `tracing::error!`
2. No logger construction or initialization needed
3. Add `tracing` to `[dependencies]` in Cargo.toml

### APM → OTEL

When the artifacts say the source uses an APM library:

1. Annotate all handler functions with `#[omnia_wasi_otel::instrument]`
2. Use `tracing::info!(monotonic_counter.X = 1)` for counters
3. Use `tracing::info!(gauge.X = value)` for gauges
4. Map metric names from the artifacts' Metrics & Observability section
