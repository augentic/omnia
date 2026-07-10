//! Seam test for `wasi:blobstore`: drive the `blobstore` example guest over the
//! real `wasi:http` boundary and confirm the blob landed in the shared backend.
//!
//! The guest writes the request body to a container via a streaming
//! `OutgoingValue`, reads it back through an `IncomingValue`, asserts the
//! round-trip, and echoes the parsed JSON. A probe handle onto the shared
//! backend (clones share the store) then reads the object host-side, proving
//! the write reached the host store rather than merely returning `200`.
//!
//! The guest is built automatically on first [`find_guest`] call; the test skips locally when
//! it is absent and fails under CI so the pipeline never passes vacuously.

#![cfg(not(target_arch = "wasm32"))]

use std::sync::Arc;

use anyhow::{Context as _, Result};
use omnia::wasmtime_wasi::ResourceTable;
use omnia::{Backend as _, DeploymentBuilder, HasHttp, MountRegistry, Runtime, StoreCtx};
use omnia_testkit::{find_guest, http};
use omnia_wasi_blobstore::{BlobstoreDefault, HasBlobstore, WasiBlobstore, WasiBlobstoreCtx};
use omnia_wasi_http::{HttpDefault, WasiHttp, WasiHttpCtxView};
use omnia_wasi_otel::{HasOtel, OtelDefault, WasiOtel, WasiOtelCtx};

/// The `examples/blobstore` backend bundle: `wasi:http` + `wasi:otel` +
/// `wasi:blobstore`.
#[derive(Clone)]
struct Bundle {
    http: HttpDefault,
    otel: OtelDefault,
    blobstore: BlobstoreDefault,
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

impl HasBlobstore for Bundle {
    fn blobstore_ctx(&mut self) -> &mut dyn WasiBlobstoreCtx {
        &mut self.blobstore
    }
}

/// Build the runtime, returning it plus a probe handle onto the shared
/// blobstore backend (clones share the store `Arc`, so this handle observes
/// the guest's writes).
async fn runtime() -> Result<Option<(Runtime<Bundle>, BlobstoreDefault)>> {
    let Some(wasm) = find_guest("blobstore_wasm.wasm") else {
        return Ok(None);
    };

    let bundle = Bundle {
        http: HttpDefault::connect().await.context("connecting http")?,
        otel: OtelDefault::connect().await.context("connecting otel")?,
        blobstore: BlobstoreDefault::connect().await.context("connecting blobstore")?,
    };
    let store_probe = bundle.blobstore.clone();

    let mut deployment =
        DeploymentBuilder::new().wasm(wasm).build::<StoreCtx<Bundle>>().await.context("build")?;
    deployment.host::<WasiHttp, Bundle>().context("link http")?;
    deployment.host::<WasiOtel, Bundle>().context("link otel")?;
    deployment.host::<WasiBlobstore, Bundle>().context("link blobstore")?;
    let registry = deployment.into_registry().context("assemble registry")?;

    let runtime = Runtime::from_parts(
        Arc::new(registry),
        Vec::new(),
        Arc::new(MountRegistry::default()),
        bundle,
    );
    Ok(Some((runtime, store_probe)))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn write_then_read() -> Result<()> {
    let Some((runtime, blobstore)) = runtime().await? else {
        return Ok(());
    };

    let response = http::post(&runtime, "/", r#"{"blob":"payload"}"#).await?;
    assert!(response.status().is_success(), "guest completes the blob write/read round-trip");
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(response.body())?,
        serde_json::json!({ "blob": "payload" }),
        "the guest echoes the blob it stored and read back"
    );

    // The blob written by the guest must be visible on the shared backend.
    let container =
        blobstore.get_container("container".to_string()).await.context("probe container")?;
    let data = container
        .get_data("request".to_string(), 0, 0)
        .await
        .context("probe object")?
        .context("object `request` missing from the host store")?;
    assert_eq!(data, br#"{"blob":"payload"}"#, "the guest's blob reached the host store intact");

    Ok(())
}
