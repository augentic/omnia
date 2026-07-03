//! Seam test for multi-guest HTTP routing: one server fronts two guests, and
//! `[[route.http]]` prefixes select the guest per request by longest match.
//!
//! Building the `http-routing` example deployment from a manifest and driving
//! `/a` and `/b` proves the trigger router resolves each path to the right guest
//! and that guest's response returns — the `omnia.toml` routing contract, minus
//! the TCP socket.
//!
//! The guests are built automatically on first [`find_guest`] call; the test skips locally
//! when they are absent and fails under CI so the pipeline never passes
//! vacuously.

#![cfg(not(target_arch = "wasm32"))]

use std::sync::Arc;

use anyhow::{Context as _, Result};
use omnia::wasmtime_wasi::ResourceTable;
use omnia::{Backend as _, DeploymentBuilder, HasHttp, MountRegistry, Runtime, StoreCtx};
use omnia_testkit::{find_guest, http, temp_manifest};
use omnia_wasi_http::{HttpDefault, WasiHttp, WasiHttpCtxView};
use omnia_wasi_otel::{HasOtel, OtelDefault, WasiOtel, WasiOtelCtx};

/// The `examples/http-routing` backend bundle: `wasi:http` + `wasi:otel`.
#[derive(Clone)]
struct Bundle {
    http: HttpDefault,
    otel: OtelDefault,
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

async fn runtime() -> Result<Option<Runtime<Bundle>>> {
    let (Some(guest_a), Some(guest_b)) = (
        find_guest("http_routing_a_wasm.wasm"),
        find_guest("http_routing_b_wasm.wasm"),
    ) else {
        return Ok(None);
    };

    // Mirror examples/http-routing/omnia.toml with absolute source paths so the
    // manifest resolves regardless of the working directory.
    let manifest = temp_manifest(&format!(
        "[[guest]]\n\
         id = \"a\"\n\
         source.path = \"{a}\"\n\n\
         [[guest]]\n\
         id = \"b\"\n\
         source.path = \"{b}\"\n\n\
         [[route.http]]\n\
         prefix = \"/a\"\n\
         guest = \"a\"\n\n\
         [[route.http]]\n\
         prefix = \"/b\"\n\
         guest = \"b\"\n",
        a = guest_a.display(),
        b = guest_b.display(),
    ))?;

    let bundle = Bundle {
        http: HttpDefault::connect().await.context("connecting http")?,
        otel: OtelDefault::connect().await.context("connecting otel")?,
    };

    let mut deployment = DeploymentBuilder::new()
        .config(manifest.path().to_path_buf())
        .build::<StoreCtx<Bundle>>()
        .await
        .context("build")?;
    deployment.host::<WasiHttp, Bundle>().context("link http")?;
    deployment.host::<WasiOtel, Bundle>().context("link otel")?;
    let registry = deployment.into_registry().context("assemble registry")?;

    Ok(Some(Runtime::from_parts(
        Arc::new(registry),
        Vec::new(),
        Arc::new(MountRegistry::default()),
        bundle,
    )))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn prefix_routing() -> Result<()> {
    let Some(runtime) = runtime().await? else {
        return Ok(());
    };

    let a = http::get(&runtime, "/a").await?;
    assert!(a.status().is_success(), "/a routes to a guest");
    assert!(
        String::from_utf8_lossy(a.body()).contains("guest a"),
        "/a reached guest a: {:?}",
        a.body()
    );

    let b = http::get(&runtime, "/b").await?;
    assert!(b.status().is_success(), "/b routes to a guest");
    assert!(
        String::from_utf8_lossy(b.body()).contains("guest b"),
        "/b reached guest b: {:?}",
        b.body()
    );

    Ok(())
}
