# Omnia WASI HTTP

This crate provides the HTTP interface for the Omnia runtime.

## Interface

Implements the `wasi:http` WIT interface (WASI Preview 2).

## Backend

Uses `hyper` and `axum` to handle outgoing requests and incoming server connections.

## Usage

Add this crate to your `Cargo.toml` and use it in your runtime configuration:

```rust,ignore
use omnia::runtime;
use omnia_wasi_http::WasiHttpCtx;

omnia::runtime!({
    "http": WasiHttpCtx,
});
```

## Outbound Resilience

All outbound HTTP requests pass through three resilience layers — **timeout**, **retry**, and **circuit breaker** — managed entirely on the host side. Guest crate authors interact with a typed `OutboundPolicy` struct; the underlying header encoding is invisible.

### The Three Layers

1. **Timeout** — Every outbound request has a total time budget. The guest can override it per-request; otherwise the host default (`HTTP_RESPONSE_TIMEOUT_MS`) applies. Retries share this budget.

2. **Retry** — GET, HEAD, and OPTIONS requests are retried on transient failures (timeout, connection error, 429, 502, 503, 504). POST/PUT/PATCH/DELETE are never retried. Delays use jittered exponential backoff, and 429 responses with a `Retry-After` header use that value instead.

3. **Circuit Breaker** — A windowed three-state breaker (OFF → ON → `HALF_ON` → OFF) protects against sustained upstream failures. Faults only accumulate within a rolling time window, so sporadic errors hours apart never trip the breaker.

### WASI Boundary

Rust `http::Request` extensions do not cross the WASI component model boundary. `OutboundPolicy` is serialized into `X-Omnia-Timeout-Ms` and `X-Omnia-Upstream` headers inside `guest/outgoing.rs`, and the host strips them before forwarding. The upstream never sees these headers.

### Circuit Breaker States

```text
OFF ──(faults ≥ threshold within window)──► ON ──(reset period elapsed)──► HALF_ON
 ▲                                                                            │
 └──────────(successes ≥ switch_off_threshold)────────────────────────────────┘
                                                    │
                                          (any fault) ──► ON
```

- **OFF**: Requests flow normally. Faults are tracked in a rolling window.
- **ON**: All requests are rejected immediately. After `reset_period_ms`, transitions to `HALF_ON`.
- **HALF_ON**: A limited number of probe requests are allowed. If enough succeed, the breaker closes (OFF). Any single fault sends it back to ON.

### Fault Window

The fault counter resets to zero if no new fault arrives within `fault_window_ms`. This prevents slow trickles of errors from accumulating. Example: one timeout per hour over 10 hours = 10 faults, but each is isolated in its own window and never trips the breaker.

### Configuration

| Variable                       | Default             | Description                                                                 |
| ------------------------------ | ------------------- | --------------------------------------------------------------------------- |
| `HTTP_OUTBOUND_RESILIENCE`     | `false`             | Enable retry and circuit breaker. When `false`, only timeout is applied.    |
| `HTTP_RESPONSE_TIMEOUT_MS`     | `0`                 | Default total timeout budget (ms). `0` = no timeout (infinite).             |
| `HTTP_RETRY_MAX`               | `2`                 | Max retry attempts (GET/HEAD/OPTIONS only). Ignored unless resilience = on. |
| `HTTP_RETRY_BASE_DELAY_MS`     | `100`               | Exponential backoff base delay (ms)                                         |
| `HTTP_RETRY_CAP_DELAY_MS`      | `1000`              | Max backoff delay (ms)                                                      |
| `HTTP_CB_SWITCH_ON_THRESHOLD`  | `10`                | Faults within window to trip breaker                                        |
| `HTTP_CB_SWITCH_OFF_THRESHOLD` | `5`                 | Probe successes to close breaker                                            |
| `HTTP_CB_RESET_PERIOD_MS`      | `10000`             | Time in ON before allowing probes (ms)                                      |
| `HTTP_CB_FAULT_WINDOW_MS`      | `30000`             | Rolling window for fault accumulation (ms)                                  |
| `HTTP_CB_BUCKETS`              | `""`                | Comma-separated bucket names                                                |
| `HTTP_CB_{NAME}_*`             | _(inherits global)_ | Per-bucket threshold overrides                                              |

> **Note:** Retry and circuit breaker env vars (`HTTP_RETRY_*`, `HTTP_CB_*`) are ignored unless `HTTP_OUTBOUND_RESILIENCE=true`. Timeout (`HTTP_RESPONSE_TIMEOUT_MS`) always applies when non-zero, regardless of the resilience toggle.

### Buckets

Declare named breaker buckets via `HTTP_CB_BUCKETS=monitoring,messaging`. Each bucket gets its own independent circuit breaker. Requests are routed to a bucket by:

1. Explicit `upstream` field in `OutboundPolicy` (highest priority)
2. First path segment of the URL (e.g., `/monitoring/v2/foo` → `monitoring`)
3. Default breaker (fallback)

Per-bucket overrides: set `HTTP_CB_MONITORING_SWITCH_ON_THRESHOLD=3` to give the `monitoring` bucket a custom threshold.

### Guest Usage

```rust,ignore
use omnia_wasi_http::OutboundPolicy;

// Simple — use defaults
let response = provider.fetch(request).await?;

// With timeout override
request.extensions_mut().insert(OutboundPolicy {
    timeout_ms: Some(5000),
    upstream: None, // auto-derived from URL path
});
let response = provider.fetch(request).await?;

// With explicit bucket
request.extensions_mut().insert(OutboundPolicy {
    timeout_ms: Some(10_000),
    upstream: Some("monitoring".into()),
});
let response = provider.fetch(request).await?;
```

### Error Handling

When the circuit breaker is open, the guest receives an `ErrorCode::InternalError` with the message `"circuit breaker open: {bucket_name}"`. Guest crates should handle this gracefully — e.g., return cached data or a degraded response.

## License

MIT OR Apache-2.0
