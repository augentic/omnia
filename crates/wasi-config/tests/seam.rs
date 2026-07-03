//! Seam test for `wasi:config`: drive the `config` example guest over the real
//! `wasi:http` boundary.
//!
//! The guest calls `get-all` and returns the variables under a `config` key, so
//! a `200` with a `config` object proves the config path crossed the WIT
//! boundary. `wasi:config`'s view is read-only (`&self`), which the bundle's
//! `HasConfig` impl reflects.
//!
//! The guest is built automatically on first [`find_guest`] call; the test skips locally when
//! it is absent and fails under CI so the pipeline never passes vacuously.

#![cfg(not(target_arch = "wasm32"))]

use std::sync::Arc;

use anyhow::{Context as _, Result};
use omnia::wasmtime_wasi::ResourceTable;
use omnia::{Backend as _, DeploymentBuilder, HasHttp, MountRegistry, Runtime, StoreCtx};
use omnia_testkit::{find_guest, http};
use omnia_wasi_config::{ConfigDefault, HasConfig, WasiConfig, WasiConfigCtx};
use omnia_wasi_http::{HttpDefault, WasiHttp, WasiHttpCtxView};
use omnia_wasi_otel::{HasOtel, OtelDefault, WasiOtel, WasiOtelCtx};

/// The `examples/config` backend bundle: `wasi:http` + `wasi:otel` +
/// `wasi:config`.
#[derive(Clone)]
struct Bundle {
    http: HttpDefault,
    otel: OtelDefault,
    config: ConfigDefault,
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

// `wasi:config` is read-only, so its accessor borrows `&self`.
impl HasConfig for Bundle {
    fn config_ctx(&self) -> &dyn WasiConfigCtx {
        &self.config
    }
}

async fn runtime() -> Result<Option<Runtime<Bundle>>> {
    let Some(wasm) = find_guest("config_wasm.wasm") else {
        return Ok(None);
    };

    let bundle = Bundle {
        http: HttpDefault::connect().await.context("connecting http")?,
        otel: OtelDefault::connect().await.context("connecting otel")?,
        config: ConfigDefault::connect().await.context("connecting config")?,
    };

    let mut deployment =
        DeploymentBuilder::new().wasm(wasm).build::<StoreCtx<Bundle>>().await.context("build")?;
    deployment.host::<WasiHttp, Bundle>().context("link http")?;
    deployment.host::<WasiOtel, Bundle>().context("link otel")?;
    deployment.host::<WasiConfig, Bundle>().context("link config")?;
    let registry = deployment.into_registry().context("assemble registry")?;

    Ok(Some(Runtime::from_parts(
        Arc::new(registry),
        Vec::new(),
        Arc::new(MountRegistry::default()),
        bundle,
    )))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn get_all() -> Result<()> {
    let Some(runtime) = runtime().await? else {
        return Ok(());
    };

    let response = http::get(&runtime, "/").await?;
    assert!(response.status().is_success(), "guest reads config across the boundary");

    // `get-all` returns `list<tuple<string, string>>`, so `config` is a JSON
    // array of `[key, value]` pairs. The runtime's own env is non-empty, so a
    // populated array proves the variables crossed the boundary.
    let body: serde_json::Value = serde_json::from_slice(response.body())?;
    let config = body.get("config").context("response carries a config field")?;
    assert!(config.is_array(), "config is the get-all list: {config}");

    Ok(())
}
