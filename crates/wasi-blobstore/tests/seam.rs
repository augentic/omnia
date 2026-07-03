//! Seam test for `wasi:blobstore`: drive the `blobstore` example guest over the
//! real `wasi:http` boundary.
//!
//! The guest writes the request body to a container via a streaming
//! `OutgoingValue`, reads it back through an `IncomingValue`, asserts the
//! round-trip, and echoes the parsed JSON — so a `200` with the same body
//! proves the create/write/read blob path crossed the WIT boundary intact.
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

async fn runtime() -> Result<Option<Runtime<Bundle>>> {
    let Some(wasm) = find_guest("blobstore_wasm.wasm") else {
        return Ok(None);
    };

    let bundle = Bundle {
        http: HttpDefault::connect().await.context("connecting http")?,
        otel: OtelDefault::connect().await.context("connecting otel")?,
        blobstore: BlobstoreDefault::connect().await.context("connecting blobstore")?,
    };

    let mut deployment =
        DeploymentBuilder::new().wasm(wasm).build::<StoreCtx<Bundle>>().await.context("build")?;
    deployment.host::<WasiHttp, Bundle>().context("link http")?;
    deployment.host::<WasiOtel, Bundle>().context("link otel")?;
    deployment.host::<WasiBlobstore, Bundle>().context("link blobstore")?;
    let registry = deployment.into_registry().context("assemble registry")?;

    Ok(Some(Runtime::from_parts(
        Arc::new(registry),
        Vec::new(),
        Arc::new(MountRegistry::default()),
        bundle,
    )))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn write_then_read() -> Result<()> {
    let Some(runtime) = runtime().await? else {
        return Ok(());
    };

    let response = http::post(&runtime, "/", r#"{"blob":"payload"}"#).await?;
    assert!(response.status().is_success(), "guest completes the blob write/read round-trip");
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(response.body())?,
        serde_json::json!({ "blob": "payload" }),
        "the guest echoes the blob it stored and read back"
    );

    Ok(())
}
