//! Seam test for `wasi:docstore`: drive the `docstore` example guest over the
//! real `wasi:http` boundary with a create-then-read round-trip.
//!
//! A `POST /stops` inserts a document and a follow-up `GET /stops/{id}` reads it
//! back through a freshly instantiated guest sharing the same backend — proving
//! both the insert and get paths cross the WIT boundary and that the write is
//! durable across instance-per-call invocations.
//!
//! The guest is built by `cargo make build-guests`; the test skips locally when
//! it is absent and fails under CI so the pipeline never passes vacuously.

#![cfg(not(target_arch = "wasm32"))]

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context as _, Result};
use omnia::wasmtime_wasi::ResourceTable;
use omnia::{Backend as _, DeploymentBuilder, HasHttp, MountRegistry, Runtime, StoreCtx};
use omnia_testkit::{find_guest, http};
use omnia_wasi_docstore::{DocStoreDefault, HasDocStore, WasiDocStore, WasiDocStoreCtx};
use omnia_wasi_http::{HttpDefault, WasiHttp, WasiHttpCtxView};
use omnia_wasi_otel::{HasOtel, OtelDefault, WasiOtel, WasiOtelCtx};

/// The `examples/docstore` backend bundle: `wasi:http` + `wasi:otel` +
/// `wasi:docstore`.
#[derive(Clone)]
struct Bundle {
    http: HttpDefault,
    otel: OtelDefault,
    docstore: DocStoreDefault,
}

impl HasHttp for Bundle {
    fn http_view<'a>(&'a mut self, table: &'a mut ResourceTable) -> WasiHttpCtxView<'a> {
        self.http.as_view(table)
    }
}

impl HasOtel for Bundle {
    fn otel_ctx(&mut self) -> &mut dyn WasiOtelCtx {
        &mut self.otel
    }
}

impl HasDocStore for Bundle {
    fn docstore_ctx(&mut self) -> &mut dyn WasiDocStoreCtx {
        &mut self.docstore
    }
}

async fn runtime() -> Result<Option<Runtime<Bundle>>> {
    let Some(wasm) = find_guest("docstore_wasm.wasm", "cargo make build-guests") else {
        return Ok(None);
    };

    let bundle = Bundle {
        http: HttpDefault::connect().await.context("connecting http")?,
        otel: OtelDefault::connect().await.context("connecting otel")?,
        docstore: DocStoreDefault::connect().await.context("connecting docstore")?,
    };

    let mut deployment =
        DeploymentBuilder::new().wasm(wasm).build::<StoreCtx<Bundle>>().await.context("build")?;
    deployment.host::<WasiHttp, Bundle>().context("link http")?;
    deployment.host::<WasiOtel, Bundle>().context("link otel")?;
    deployment.host::<WasiDocStore, Bundle>().context("link docstore")?;
    let registry = deployment.into_registry().context("assemble registry")?;

    Ok(Some(Runtime::from_parts(
        Arc::new(registry),
        Vec::new(),
        Arc::new(MountRegistry::default()),
        bundle,
    )))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn insert_then_get() -> Result<()> {
    let Some(runtime) = runtime().await? else {
        return Ok(());
    };

    // A unique id keeps the default (persistent) PoloDB from rejecting the
    // insert as a duplicate across runs.
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos();
    let id = format!("stop-{}-{nanos}", std::process::id());

    let create = http::post_json(
        &runtime,
        "/stops",
        format!(r#"{{"id":"{id}","stop_name":"Central","stop_lat":1.5,"stop_lon":2.5}}"#),
    )
    .await?;
    assert!(create.status().is_success(), "guest inserts the document across the boundary");

    let fetched = http::get(&runtime, &format!("/stops/{id}")).await?;
    assert!(fetched.status().is_success(), "guest reads the document back");
    let body: serde_json::Value = serde_json::from_slice(fetched.body())?;
    assert_eq!(body["id"], serde_json::json!(id), "the id round-trips");
    assert_eq!(body["stop"]["stop_name"], serde_json::json!("Central"), "the document round-trips");

    Ok(())
}
