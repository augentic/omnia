//! A `monotonic_counter.*` tracing field reaches the host as a metric:
//! `command!` owns the telemetry lifecycle, so two increments accumulate in
//! the guest's `MetricsLayer` and the flush at the end of the run exports a
//! single `hits` sum.

#![cfg(target_arch = "wasm32")]

omnia_guest::command!(scenario);

async fn scenario() {
    // ERROR-level: the events must pass the guest's default `EnvFilter` (the
    // test environment sets no `RUST_LOG`).
    tracing::error!(monotonic_counter.hits = 1);
    tracing::error!(monotonic_counter.hits = 1);
}
