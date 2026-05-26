#![allow(missing_docs)]
//! Benchmarks for the circuit breaker state machine.
//!
//! Scenarios:
//! 1. **Steady-state**: `check` + `record_success` on a Closed breaker (normal traffic)
//! 2. **Contended (threads)**: N OS threads hammering `check` + `record_failure` (mutex micro-benchmark)
//! 3. **Contended (guests)**: N async tasks on a fixed 4-worker Tokio runtime (realistic deployment)
//! 4. **Resolve**: `BucketRegistry::resolve` lookup by name
//! 5. **Failure accumulation**: `record_failure` on a Closed breaker (fault-window bookkeeping)

use std::hint::black_box;
use std::sync::Arc;
use std::time::Duration;

use criterion::{Criterion, criterion_group, criterion_main};
use omnia_wasi_http::{BreakerConfig, BucketRegistry, CircuitBreaker};

const fn steady_state_config() -> BreakerConfig {
    BreakerConfig {
        trip_threshold: u32::MAX,
        recovery_threshold: 10,
        reset_period: Duration::from_secs(60),
        fault_window: Duration::from_secs(120),
    }
}

// ---------------------------------------------------------------------------
// 1. Steady-state: check + record_success on a Closed breaker
// ---------------------------------------------------------------------------

fn bench_steady_state(c: &mut Criterion) {
    let cb = CircuitBreaker::new("steady", steady_state_config());

    c.bench_function("steady_state_check", |b| {
        b.iter(|| black_box(cb.check()));
    });

    c.bench_function("steady_state_record_success", |b| {
        b.iter(|| cb.record_success());
    });

    c.bench_function("steady_state_round_trip", |b| {
        b.iter(|| {
            black_box(cb.check());
            cb.record_success();
        });
    });

    let reg = BucketRegistry::new(vec!["upstream".to_owned()], &steady_state_config()).unwrap();
    c.bench_function("steady_state_full_path", |b| {
        b.iter(|| {
            let resolved = black_box(reg.resolve(Some("upstream"))).unwrap().unwrap();
            black_box(resolved.breaker.check());
            resolved.breaker.record_success();
        });
    });
}

// ---------------------------------------------------------------------------
// 2. Contended: N threads hammering check + record on a shared breaker
// ---------------------------------------------------------------------------

fn bench_contended(c: &mut Criterion) {
    let mut group = c.benchmark_group("contended");

    for threads in [2, 4, 8] {
        group.bench_function(format!("{threads}t_check_record"), |b| {
            let cb = Arc::new(CircuitBreaker::new("contended", steady_state_config()));
            b.iter(|| {
                std::thread::scope(|s| {
                    for _ in 0..threads {
                        let cb = &cb;
                        s.spawn(move || {
                            for _ in 0..1_000 {
                                black_box(cb.check());
                                cb.record_failure();
                            }
                        });
                    }
                });
            });
        });
    }

    group.finish();
}

// ---------------------------------------------------------------------------
// 3. Contended (guests): N async tasks on a fixed-size Tokio runtime
// ---------------------------------------------------------------------------

fn bench_contended_guests(c: &mut Criterion) {
    let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(4).build().unwrap();

    let mut group = c.benchmark_group("contended_guests");
    let config = steady_state_config();

    for guests in [10, 50, 100, 500] {
        group.bench_function(format!("{guests}_guests"), |b| {
            let reg = Arc::new(BucketRegistry::new(vec!["upstream".to_owned()], &config).unwrap());
            b.iter(|| {
                rt.block_on(async {
                    let mut handles = Vec::with_capacity(guests);
                    for _ in 0..guests {
                        let reg = Arc::clone(&reg);
                        handles.push(tokio::spawn(async move {
                            for _ in 0..100 {
                                let resolved =
                                    black_box(reg.resolve(Some("upstream"))).unwrap().unwrap();
                                black_box(resolved.breaker.check());
                                resolved.breaker.record_success();
                            }
                        }));
                    }
                    for h in handles {
                        h.await.unwrap();
                    }
                });
            });
        });
    }

    group.finish();
}

// ---------------------------------------------------------------------------
// 4. Resolve: BucketRegistry lookup by name
// ---------------------------------------------------------------------------

fn bench_resolve(c: &mut Criterion) {
    let names: Vec<String> = (0..50).map(|i| format!("bucket-{i}")).collect();
    let reg = BucketRegistry::new(names, &BreakerConfig::default()).unwrap();

    let mut group = c.benchmark_group("resolve");

    group.bench_function("hit", |b| {
        b.iter(|| black_box(reg.resolve(Some("bucket-25"))));
    });

    group.bench_function("none", |b| {
        b.iter(|| black_box(reg.resolve(None)));
    });

    group.bench_function("miss", |b| {
        b.iter(|| black_box(reg.resolve(Some("nonexistent"))));
    });

    group.finish();
}

// ---------------------------------------------------------------------------
// 5. Failure accumulation: record_failure doing fault-window bookkeeping
// ---------------------------------------------------------------------------

fn bench_failure_accumulation(c: &mut Criterion) {
    let mut group = c.benchmark_group("failure_accumulation");

    group.bench_function("within_window", |b| {
        let cb = CircuitBreaker::new("fail", steady_state_config());
        b.iter(|| cb.record_failure());
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_steady_state,
    bench_contended,
    bench_contended_guests,
    bench_resolve,
    bench_failure_accumulation,
);
criterion_main!(benches);
