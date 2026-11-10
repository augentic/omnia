//! What a guest store costs: building one over the deployment's backend
//! bundle, and assembling one host's view of it.
//!
//! The runtime links twelve hosts over the in-memory defaults. Run with
//! `cargo bench -p omnia-test --features host --bench store`.

use std::hint::black_box;
use std::time::Instant;

use anyhow::Result;
use omnia::{DeploymentBuilder, Runtime};
use omnia_test::host::Backends;
use omnia_wasi_blobstore::WasiBlobstore;
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
const STORES_PER_ROUND: u32 = 2_000;
const VIEWS_PER_ROUND: u32 = 2_000_000;

#[tokio::main]
#[expect(clippy::print_stdout, reason = "a bench reports on stdout")]
async fn main() -> Result<()> {
    let runtime = boot(Backends::defaults().await).await?;

    let mut store = Samples::default();
    let mut view = Samples::default();
    for round in 0..=ROUNDS {
        let store_nanos = time_store(&runtime);
        let view_nanos = time_view(&runtime);
        // the first round warms the allocator and the caches
        if round > 0 {
            store.0.push(store_nanos);
            view.0.push(view_nanos);
        }
    }

    println!("nanoseconds per operation, median of {ROUNDS} rounds (fastest round)");
    println!("store, built and dropped   {store}");
    println!("view of `wasi:keyvalue`    {view}");

    runtime.shutdown();
    Ok(())
}

async fn boot(backends: Backends) -> Result<Runtime<Backends>> {
    let mut deployment = DeploymentBuilder::new().dynamic().build::<Backends>().await?;
    deployment
        .host::<WasiBlobstore>()?
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

fn time_store(runtime: &Runtime<Backends>) -> f64 {
    time(STORES_PER_ROUND, || drop(black_box(runtime.build_store(runtime.store()))))
}

fn time_view(runtime: &Runtime<Backends>) -> f64 {
    let mut store = runtime.store();
    time(VIEWS_PER_ROUND, || {
        let view = black_box(&mut store).view::<WasiKeyValue>();
        black_box(view.ctx);
    })
}

fn time(operations: u32, mut operation: impl FnMut()) -> f64 {
    let start = Instant::now();
    for _ in 0..operations {
        operation();
    }
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
