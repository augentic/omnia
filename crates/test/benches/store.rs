//! What entering a guest costs: leasing a slot of the concurrency gate,
//! building its store over the deployment's backend bundle and instantiating
//! it, and assembling one host's view of the store.
//!
//! The runtime links twelve hosts over the in-memory defaults, for two
//! guests over one component. One declares no grants. The other's grants
//! narrow the environment and mounts its store is built with. Run with
//! `cargo bench -p omnia-test --features host --bench store`.

use std::hint::black_box;
use std::sync::Arc;
use std::time::Instant;

use anyhow::Result;
use omnia::wasmtime::AsContextMut as _;
use omnia::{DeploymentBuilder, Grants, Guest, GuestEntry, GuestId, Manifest, Runtime};
use omnia_test::host::{Backends, Scratch, scratch};
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
    let data = scratch();
    let runtime = boot(Backends::defaults().await, &data).await?;
    let plain = runtime.guest(&GuestId::from("plain")).await?;
    let granted = runtime.guest(&GuestId::from("granted")).await?;

    let mut entry = Samples::default();
    let mut granted_entry = Samples::default();
    let mut view = Samples::default();
    for round in 0..=ROUNDS {
        let entry_nanos = time_enter(&runtime, &plain).await?;
        let granted_nanos = time_enter(&runtime, &granted).await?;
        let view_nanos = time_view(&runtime, &plain).await?;
        // the first round warms the allocator and the caches
        if round > 0 {
            entry.0.push(entry_nanos);
            granted_entry.0.push(granted_nanos);
            view.0.push(view_nanos);
        }
    }

    println!("nanoseconds per operation, median of {ROUNDS} rounds (fastest round)");
    println!("guest entered and dropped          {entry}");
    println!("granted guest entered and dropped  {granted_entry}");
    println!("view of `omnia:keyvalue`           {view}");

    runtime.shutdown();
    Ok(())
}

async fn boot(backends: Backends, data: &Scratch) -> Result<Runtime<Backends>> {
    let bytes = std::fs::read(test_programs::LINK_ECHOER)?;
    let grants = Grants::all().env(["CARGO_PKG_*"]).mounts(["data"]);
    let manifest = Manifest::new()
        .guest(GuestEntry::new("plain", bytes.clone()))
        .guest(GuestEntry::new("granted", bytes).grants(grants))
        .mounts([data.mount_as("data", false)])
        .inherit(["*"]);
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

async fn time_enter(runtime: &Runtime<Backends>, guest: &Arc<Guest<Backends>>) -> Result<f64> {
    let start = Instant::now();
    for _ in 0..ENTRIES_PER_ROUND {
        let lease = runtime.lease(Arc::clone(guest)).await;
        lease
            .invoke(async |entered, instance, _bound| drop(black_box((entered, instance))))
            .await?;
    }
    Ok(nanos_per(start, ENTRIES_PER_ROUND))
}

async fn time_view(runtime: &Runtime<Backends>, guest: &Arc<Guest<Backends>>) -> Result<f64> {
    let lease = runtime.lease(Arc::clone(guest)).await;
    let nanos = lease
        .invoke(async |mut entered, _instance, _bound| {
            let mut store = entered.as_context_mut();
            time(VIEWS_PER_ROUND, || {
                let view = black_box(store.data_mut()).view::<WasiKeyValue>();
                black_box(view.client);
            })
        })
        .await?;
    Ok(nanos)
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
