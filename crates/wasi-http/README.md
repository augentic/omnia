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

3. **Circuit Breaker** — A three-state breaker (`Closed` → `Open` → `HalfOpen` → `Closed`) protects against sustained upstream failures. Only 5xx responses, timeouts, and connection errors count as faults — 429 (rate limiting) is handled by the retry layer and does not trip the breaker. Faults only accumulate within a fixed fault window, so sporadic errors hours apart never trip the breaker.

### Retry vs Breaker Interaction

The retry loop runs *inside* the circuit breaker. The breaker only sees the final outcome after retries are exhausted — intermediate failures during retries do not count as breaker faults.

| Outcome          | Retried?          | Faults breaker? |
| ---------------- | ----------------- | --------------- |
| 429              | Yes               | No              |
| 500              | No                | Yes             |
| 502              | Yes               | Yes             |
| 503              | Yes               | Yes             |
| 504              | Yes               | Yes             |
| Timeout          | Yes               | Yes             |
| Connection error | Yes               | Yes             |

- **429** is retried (with `Retry-After` support) but does not fault the breaker — it's a rate-limiting signal, not a failure.
- **500** faults the breaker but is not retried — it typically indicates a bug in the upstream rather than a transient issue.
- Retry only applies to GET, HEAD, and OPTIONS. Non-idempotent methods (POST, PUT, PATCH, DELETE) are never retried but still record breaker faults on 5xx/timeout/connection error.

### WASI Boundary

Rust `http::Request` extensions do not cross the WASI component model boundary. `OutboundPolicy` is serialized into `X-Omnia-Timeout-Ms` and `X-Omnia-Upstream` headers inside `guest/outgoing.rs`, and the host strips them before forwarding. The upstream never sees these headers.

### Circuit Breaker States

```text
Closed ──(faults ≥ threshold within window)──► Open ──(reset period elapsed)──► HalfOpen
  ▲                                                                                │
  └──────────(successes ≥ switch_off_threshold)────────────────────────────────────┘
                                                       │
                                             (any fault) ──► Open
```

- **Closed**: Requests flow normally. Faults are tracked within a fixed fault window.
- **Open**: All requests are rejected immediately. After `reset_period_ms`, transitions to `HalfOpen`.
- **HalfOpen**: A limited number of probe requests are allowed. If enough succeed, the breaker closes (`Closed`). Any single fault sends it back to `Open`.

### Fault Window

Faults are counted within a fixed window anchored at the first fault. When the window expires (elapsed > `fault_window_ms`), the counter resets. This prevents slow trickles of errors from accumulating. Example: one timeout per hour over 10 hours = 10 faults, but each is isolated in its own window and never trips the breaker.

### Configuration

| Variable                       | Default             | Description                                                                 |
| ------------------------------ | ------------------- | --------------------------------------------------------------------------- |
| `HTTP_OUTBOUND_RESILIENCE`     | `false`             | Enable retry and circuit breaker. When `false`, only timeout is applied.    |
| `HTTP_RESPONSE_TIMEOUT_MS`     | `0`                 | Default total timeout budget (ms). `0` = no timeout (infinite).             |
| `HTTP_RETRY_MAX`               | `2`                 | Max retry attempts (GET/HEAD/OPTIONS only). Ignored unless resilience = on. |
| `HTTP_RETRY_BASE_DELAY_MS`     | `100`               | Exponential backoff base delay (ms)                                         |
| `HTTP_RETRY_CAP_DELAY_MS`      | `1000`              | Max backoff delay (ms)                                                      |
| `HTTP_CB_SWITCH_ON_THRESHOLD`  | `10`                | Faults within window to trip breaker                                        |
| `HTTP_CB_SWITCH_OFF_THRESHOLD` | `5`                 | Probe successes to close breaker (also caps total probes per `HalfOpen`)    |
| `HTTP_CB_RESET_PERIOD_MS`      | `10000`             | Time in `Open` before allowing probes (ms)                                  |
| `HTTP_CB_FAULT_WINDOW_MS`      | `30000`             | Fixed fault window for fault accumulation (ms)                              |
| `HTTP_CB_BUCKETS`              | `""`                | Comma-separated bucket names for isolation                                  |

> **Note:** Retry and circuit breaker env vars (`HTTP_RETRY_*`, `HTTP_CB_*`) are ignored unless `HTTP_OUTBOUND_RESILIENCE=true`. Timeout (`HTTP_RESPONSE_TIMEOUT_MS`) always applies when non-zero, regardless of the resilience toggle.

### Buckets

Declare named breaker buckets via `HTTP_CB_BUCKETS=monitoring,messaging`. Each bucket gets its own independent circuit breaker with the same global thresholds — bucket names create isolation boundaries so that failures in one upstream don't affect another.

The breaker only activates when the guest explicitly sets the `upstream` field in `OutboundPolicy` to a declared bucket name:

- **No upstream specified** — the breaker is bypassed entirely; retry and timeout still apply.
- **Known bucket** — full circuit breaker protection on that bucket.
- **Unknown bucket** — the request is rejected immediately with an error (misconfiguration).

### Guest Usage

```rust,ignore
use omnia_sdk::OutboundPolicy;

// Simple — retry + timeout only, no circuit breaker
let response = provider.fetch(request).await?;

// With timeout override, still no breaker
request.extensions_mut().insert(OutboundPolicy {
    timeout_ms: Some(5000),
    upstream: None,
});
let response = provider.fetch(request).await?;

// With explicit bucket — enables circuit breaker for this upstream
request.extensions_mut().insert(OutboundPolicy {
    timeout_ms: Some(10_000),
    upstream: Some("monitoring".into()),
});
let response = provider.fetch(request).await?;
```

### Error Handling

- When the circuit breaker is open, the guest receives an `ErrorCode::InternalError` with the message `"circuit breaker open: {bucket_name}"`.
- When the guest specifies an unknown bucket name, it receives `"unknown circuit breaker bucket: {name}"`.

Guest crates should handle these gracefully — e.g., return cached data or a degraded response.

## License

MIT OR Apache-2.0
