//! What the slot bundle costs against the static bundle: building a guest
//! store, and assembling one host's view of it.
//!
//! Both runtimes link the same twelve hosts over the same in-memory
//! defaults. Run with `cargo bench -p omnia-test --features host --bench store`.

use std::hint::black_box;
use std::time::Instant;

use anyhow::Result;
use omnia::{Backends as Bundle, DeploymentBuilder, Link, Runtime, Slots};
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

type Stores = (WasiBlobStore, WasiDocStore, WasiKeyValue, WasiSql);
type Services = (WasiConfig, WasiIdentity, WasiModel, WasiVault);
type Triggers = (WasiHttp, WasiMessaging, WasiOtel, WasiWebSocket);

// rounds alternate between the bundles, so drift lands on both alike
const ROUNDS: usize = 41;
const STORES_PER_ROUND: u32 = 2_000;
const VIEWS_PER_ROUND: u32 = 2_000_000;

#[tokio::main]
#[expect(clippy::print_stdout, reason = "a bench reports on stdout")]
async fn main() -> Result<()> {
    let defaults = Backends::defaults().await;
    let mut slots = Slots::new();
    slots.add(&defaults.blobstore)?.add(&defaults.docstore)?.add(&defaults.sql)?;
    slots.add(&defaults.config)?.add(&defaults.identity)?;
    slots.add(&defaults.model)?.add(&defaults.vault)?;
    slots.add(&defaults.http)?.add(&defaults.messaging)?;
    slots.add(&defaults.otel)?.add(&defaults.websocket)?;
    // last, so the timed view pays the longest lookup
    slots.add(&defaults.keyvalue)?;

    let fixed = boot(defaults).await?;
    let slotted = boot(slots).await?;

    let mut store = (Samples::default(), Samples::default());
    let mut view = (Samples::default(), Samples::default());
    for round in 0..=ROUNDS {
        let rounds = [
            (&mut store.0, time_store(&fixed)),
            (&mut store.1, time_store(&slotted)),
            (&mut view.0, time_view(&fixed)),
            (&mut view.1, time_view(&slotted)),
        ];
        // the first round warms the allocator and the caches
        if round > 0 {
            for (samples, nanos) in rounds {
                samples.0.push(nanos);
            }
        }
    }

    println!("nanoseconds per operation, median of {ROUNDS} rounds (fastest round)");
    println!("store, built and dropped   static {}   slots {}", store.0, store.1);
    println!("view of `wasi:keyvalue`    static {}   slots {}", view.0, view.1);
    println!(
        "slots add {:.1}% to a store and {:.1} ns to a view",
        (store.1.median() / store.0.median() - 1.0) * 100.0,
        view.1.median() - view.0.median(),
    );

    fixed.shutdown();
    slotted.shutdown();
    Ok(())
}

async fn boot<B>(backends: B) -> Result<Runtime<B>>
where
    B: Bundle,
    Stores: Link<B>,
    Services: Link<B>,
    Triggers: Link<B>,
{
    let mut deployment = DeploymentBuilder::new().dynamic().build::<B>().await?;
    deployment.hosts::<Stores>()?.hosts::<Services>()?.hosts::<Triggers>()?;
    deployment.assemble(backends).await
}

fn time_store<B: Bundle>(runtime: &Runtime<B>) -> f64 {
    time(STORES_PER_ROUND, || drop(black_box(runtime.build_store(runtime.store()))))
}

fn time_view<B>(runtime: &Runtime<B>) -> f64
where
    B: Bundle + omnia::Provides<WasiKeyValue>,
{
    let mut store = runtime.store();
    time(VIEWS_PER_ROUND, || {
        let view = black_box(&mut store).view::<WasiKeyValue>();
        black_box(view.client);
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
