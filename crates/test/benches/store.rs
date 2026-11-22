//! What entering a guest costs: building its store over the deployment's
//! backend bundle and instantiating it, and assembling one host's view of the
//! store.
//!
//! The runtime links twelve hosts over the in-memory defaults, for one
//! guest. Run with `cargo bench -p omnia-test --features host --bench store`.

use std::hint::black_box;
use std::time::Instant;

use anyhow::{Context as _, Result};
use omnia::{ChainCtx, DeploymentBuilder, Guest, Manifest, Runtime};
use omnia_test::host::Backends;
use omnia_wasi_blobstore::WasiBlobStore;
use omnia_wasi_config::WasiConfig;
use omnia_wasi_docstore::WasiDocStore;
use omnia_wasi_http::WasiHttp;
use omnia_wasi_identity::WasiIdentity;
use omnia_wasi_keyvalue::WasiKeyValue;
use omnia_wasi_messaging::WasiMessaging;
use omnia_wasi_model::WasiModel;
use omnia_wasi_otel::WasiOtel;
use omnia_wasi_sql::WasiSql;
use omnia_wasi_vault::WasiVault;
use omnia_wasi_websocket::WasiWebSocket;

const ROUNDS: usize = 41;
const ENTRIES_PER_ROUND: u32 = 2_000;
const VIEWS_PER_ROUND: u32 = 2_000_000;

#[tokio::main]
#[expect(clippy::print_stdout, reason = "a bench reports on stdout")]
async fn main() -> Result<()> {
    let runtime = boot(Backends::defaults().await).await?;
    let guest = runtime.registry().guests().next().context("the deployment runs a guest")?;

    let mut entry = Samples::default();
    let mut view = Samples::default();
    for round in 0..=ROUNDS {
        let entry_nanos = time_enter(&runtime, &guest).await?;
        let view_nanos = time_view(&runtime, &guest).await?;
        // the first round warms the allocator and the caches
        if round > 0 {
            entry.0.push(entry_nanos);
            view.0.push(view_nanos);
        }
    }

    println!("nanoseconds per operation, median of {ROUNDS} rounds (fastest round)");
    println!("guest entered and dropped  {entry}");
    println!("view of `omnia:keyvalue`   {view}");

    runtime.shutdown();
    Ok(())
}

async fn boot(backends: Backends) -> Result<Runtime<Backends>> {
    let manifest = Manifest::from_wasm(test_programs::LINK_ECHOER)?;
    let mut deployment = DeploymentBuilder::new().manifest(manifest).build::<Backends>().await?;
    deployment
        .host::<WasiBlobStore>()?
        .host::<WasiDocStore>()?
        .host::<WasiKeyValue>()?
        .host::<WasiSql>()?
        .host::<WasiConfig>()?
        .host::<WasiIdentity>()?
        .host::<WasiModel>()?
        .host::<WasiVault>()?
        .host::<WasiHttp>()?
        .host::<WasiMessaging>()?
        .host::<WasiOtel>()?
        .host::<WasiWebSocket>()?;
    deployment.assemble(backends).await
}

async fn time_enter(runtime: &Runtime<Backends>, guest: &Guest<Backends>) -> Result<f64> {
    let start = Instant::now();
    for _ in 0..ENTRIES_PER_ROUND {
        drop(black_box(runtime.enter(guest, ChainCtx::server()).await?));
    }
    Ok(nanos_per(start, ENTRIES_PER_ROUND))
}

async fn time_view(runtime: &Runtime<Backends>, guest: &Guest<Backends>) -> Result<f64> {
    let (mut store, _instance) = runtime.enter(guest, ChainCtx::server()).await?;
    Ok(time(VIEWS_PER_ROUND, || {
        let view = black_box(store.data_mut()).view::<WasiKeyValue>();
        black_box(view.client);
    }))
}

fn time(operations: u32, mut operation: impl FnMut()) -> f64 {
    let start = Instant::now();
    for _ in 0..operations {
        operation();
    }
    nanos_per(start, operations)
}

fn nanos_per(start: Instant, operations: u32) -> f64 {
    start.elapsed().as_secs_f64() * 1e9 / f64::from(operations)
}

#[derive(Default)]
struct Samples(Vec<f64>);

impl Samples {
    fn median(&self) -> f64 {
        let mut sorted = self.0.clone();
        sorted.sort_by(f64::total_cmp);
        sorted[sorted.len() / 2]
    }

    fn fastest(&self) -> f64 {
        self.0.iter().copied().fold(f64::INFINITY, f64::min)
    }
}

impl std::fmt::Display for Samples {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:>9.1} ({:.1})", self.median(), self.fastest())
    }
}
