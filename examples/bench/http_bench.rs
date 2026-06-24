//! # HTTP load-test harness
//!
//! Fires concurrent keep-alive requests at a running `http` example host and
//! reports throughput plus p50/p90/p99/p99.9 latency. It exists to gate
//! residency/MPK tuning and catch regressions in the per-request
//! instantiation path; it is self-contained (no external load tools). See
//! `examples/bench/README.md` for usage.
//!
//! Native-only. Build with `cargo build --example http-bench`.
//!
//! Configuration (environment variables):
//! - `BENCH_ADDR` (default `127.0.0.1:8080`)
//! - `BENCH_CONCURRENCY` (default `32`)
//! - `BENCH_DURATION_SECS` (default `10`)
//! - `BENCH_TIMEOUT_SECS` (default `5`)

cfg_if::cfg_if! {
    if #[cfg(not(target_arch = "wasm32"))] {

use std::env;
use std::error::Error as StdError;
use std::str::FromStr;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::header::{CONTENT_TYPE, HOST};
use hyper::{Method, Request, StatusCode};
use hyper_util::rt::TokioIo;
use tokio::net::TcpStream;
use tokio::task::JoinSet;
use tokio::time::timeout;

/// Resolved benchmark settings.
#[derive(Clone)]
struct Settings {
    addr: String,
    concurrency: usize,
    duration: Duration,
    request_timeout: Duration,
    body: Bytes,
}

/// Per-worker (later merged) results.
#[derive(Default)]
struct Stats {
    latencies_us: Vec<u64>,
    errors: u64,
}

#[tokio::main]
async fn main() -> Result<()> {
    let settings = Settings::from_env()?;
    println!(
        "load-testing http://{} with {} workers for {}s",
        settings.addr,
        settings.concurrency,
        settings.duration.as_secs(),
    );

    let deadline = Instant::now() + settings.duration;
    let mut workers = JoinSet::new();
    for _ in 0..settings.concurrency {
        let settings = settings.clone();
        workers.spawn(async move { run_worker(&settings, deadline).await });
    }

    let mut combined = Stats::default();
    while let Some(result) = workers.join_next().await {
        let stats = result.context("worker task panicked")?;
        combined.latencies_us.extend(stats.latencies_us);
        combined.errors += stats.errors;
    }

    report(&combined, settings.duration);

    if combined.latencies_us.is_empty() {
        bail!("no successful requests; is the host listening on {}?", settings.addr);
    }
    Ok(())
}

impl Settings {
    fn from_env() -> Result<Self> {
        let concurrency: usize = parse_env("BENCH_CONCURRENCY", 32)?;
        let duration_secs: u64 = parse_env("BENCH_DURATION_SECS", 10)?;
        let timeout_secs: u64 = parse_env("BENCH_TIMEOUT_SECS", 5)?;

        Ok(Self {
            addr: env::var("BENCH_ADDR").unwrap_or_else(|_| "127.0.0.1:8080".into()),
            concurrency: concurrency.max(1),
            duration: Duration::from_secs(duration_secs.max(1)),
            request_timeout: Duration::from_secs(timeout_secs.max(1)),
            // Small JSON payload accepted by the example guest's `Json` extractor.
            body: Bytes::from_static(b"{\"ping\":1}"),
        })
    }
}

/// Parse an environment variable, falling back to `default` when it is unset.
fn parse_env<T>(key: &str, default: T) -> Result<T>
where
    T: FromStr,
    T::Err: StdError + Send + Sync + 'static,
{
    match env::var(key) {
        Ok(value) => value.parse::<T>().with_context(|| format!("invalid {key}={value}")),
        Err(_) => Ok(default),
    }
}

/// Drive a single keep-alive connection in a loop until `deadline`, recording
/// per-request latency. Connection-level errors trigger a reconnect rather than
/// aborting the worker.
async fn run_worker(settings: &Settings, deadline: Instant) -> Stats {
    let mut stats = Stats::default();

    while Instant::now() < deadline {
        let Ok(stream) = TcpStream::connect(&settings.addr).await else {
            stats.errors += 1;
            continue;
        };
        let _ = stream.set_nodelay(true);

        let Ok((mut sender, conn)) =
            hyper::client::conn::http1::handshake(TokioIo::new(stream)).await
        else {
            stats.errors += 1;
            continue;
        };
        // Drive the connection's background I/O.
        let conn = tokio::spawn(async move {
            let _ = conn.await;
        });

        while Instant::now() < deadline {
            let Ok(request) = Request::builder()
                .method(Method::POST)
                .uri("/")
                .header(HOST, settings.addr.as_str())
                .header(CONTENT_TYPE, "application/json")
                .body(Full::new(settings.body.clone()))
            else {
                stats.errors += 1;
                break;
            };

            let start = Instant::now();
            match timeout(settings.request_timeout, sender.send_request(request)).await {
                Ok(Ok(response)) => {
                    let status = response.status();
                    // The body must be drained for the connection to be reused.
                    if response.into_body().collect().await.is_err() {
                        stats.errors += 1;
                        break;
                    }
                    if status == StatusCode::OK {
                        stats.latencies_us.push(elapsed_us(start));
                    } else {
                        stats.errors += 1;
                    }
                }
                // Send error or timeout: abandon this connection and reconnect.
                _ => {
                    stats.errors += 1;
                    break;
                }
            }
        }

        conn.abort();
    }

    stats
}

fn elapsed_us(start: Instant) -> u64 {
    u64::try_from(start.elapsed().as_micros()).unwrap_or(u64::MAX)
}

/// Print a human-readable summary of the run.
fn report(stats: &Stats, duration: Duration) {
    let total = stats.latencies_us.len();
    let secs = duration.as_secs_f64().max(f64::EPSILON);
    let rps = total as f64 / secs;

    println!("requests:   {total} ok, {} errors", stats.errors);
    println!("throughput: {rps:.0} req/s");

    if total == 0 {
        return;
    }

    let mut sorted = stats.latencies_us.clone();
    sorted.sort_unstable();
    let mean = sorted.iter().sum::<u64>() as f64 / total as f64;

    println!(
        "latency us: min {} p50 {} p90 {} p99 {} p99.9 {} max {} mean {mean:.0}",
        sorted[0],
        percentile(&sorted, 50.0),
        percentile(&sorted, 90.0),
        percentile(&sorted, 99.0),
        percentile(&sorted, 99.9),
        sorted[total - 1],
    );
}

/// Nearest-rank percentile over an ascending-sorted slice.
fn percentile(sorted: &[u64], p: f64) -> u64 {
    debug_assert!(!sorted.is_empty());
    let rank = (p / 100.0 * sorted.len() as f64).ceil() as usize;
    let idx = rank.saturating_sub(1).min(sorted.len() - 1);
    sorted[idx]
}

    } else {
        fn main() {}
    }
}
