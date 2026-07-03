//! Seam test for `wasi:keyvalue`: drive the `keyvalue` example guest over the
//! real `wasi:http` boundary and confirm the store round-trip landed host-side.
//!
//! The guest opens a bucket, `set`s the request body under `my_key`, then
//! `get`s it back — so a `200` proves the whole open/set/get path crossed the
//! WIT boundary without trapping, and reading the shared backend afterwards
//! proves the write actually reached the host store.
//!
//! The guest is built by `cargo make build-guests`; the test skips locally when
//! it is absent and fails under CI so the pipeline never passes vacuously.

#![cfg(not(target_arch = "wasm32"))]

use std::sync::Arc;

use anyhow::{Context as _, Result};
use omnia::wasmtime_wasi::ResourceTable;
use omnia::{Backend as _, DeploymentBuilder, HasHttp, MountRegistry, Runtime, StoreCtx};
use omnia_testkit::{find_guest, http};
use omnia_wasi_http::{HttpDefault, WasiHttp, WasiHttpCtxView};
use omnia_wasi_keyvalue::{HasKeyValue, KeyValueDefault, WasiKeyValue, WasiKeyValueCtx};
use omnia_wasi_otel::{HasOtel, OtelDefault, WasiOtel, WasiOtelCtx};

/// The example runtime's backend bundle: `wasi:http` + `wasi:otel` (the guest
/// instruments its handler) + `wasi:keyvalue`. Mirrors what `omnia::runtime!`
/// generates for `examples/keyvalue`.
#[derive(Clone)]
struct Bundle {
    http: HttpDefault,
    otel: OtelDefault,
    keyvalue: KeyValueDefault,
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

impl HasKeyValue for Bundle {
    fn keyvalue_ctx(&mut self) -> &mut dyn WasiKeyValueCtx {
        &mut self.keyvalue
    }
}

/// Build a single-guest runtime over `keyvalue_wasm.wasm`, returning the runtime
/// and a handle to the shared key-value backend (its `moka` cache is shared
/// across clones, so this handle observes the guest's writes).
async fn runtime() -> Result<Option<(Runtime<Bundle>, KeyValueDefault)>> {
    let Some(wasm) = find_guest("keyvalue_wasm.wasm", "cargo make build-guests") else {
        return Ok(None);
    };

    let bundle = Bundle {
        http: HttpDefault::connect().await.context("connecting http")?,
        otel: OtelDefault::connect().await.context("connecting otel")?,
        keyvalue: KeyValueDefault::connect().await.context("connecting keyvalue")?,
    };
    let store_probe = bundle.keyvalue.clone();

    let mut deployment =
        DeploymentBuilder::new().wasm(wasm).build::<StoreCtx<Bundle>>().await.context("build")?;
    deployment.host::<WasiHttp, Bundle>().context("link http")?;
    deployment.host::<WasiOtel, Bundle>().context("link otel")?;
    deployment.host::<WasiKeyValue, Bundle>().context("link keyvalue")?;
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
async fn set_then_get() -> Result<()> {
    let Some((runtime, store)) = runtime().await? else {
        return Ok(());
    };

    let response = http::post(&runtime, "/", "payload-value").await?;
    assert!(response.status().is_success(), "guest completes the keyvalue round-trip");

    // The guest stored the request body under `my_key` in `omnia_bucket`; the
    // shared backend must now hold that write.
    let bucket = store.open_bucket("omnia_bucket".to_owned()).await.context("open bucket")?;
    let stored = bucket.get("my_key".to_owned()).await.context("read my_key")?;
    assert_eq!(stored.as_deref(), Some(b"payload-value".as_slice()), "the write reached the host");

    Ok(())
}
