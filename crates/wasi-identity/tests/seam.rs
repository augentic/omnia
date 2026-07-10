//! Seam test for `wasi:identity`: drive the `identity` example guest through
//! the real WIT boundary against the credential-free [`IdentityStub`].
//!
//! The guest resolves an identity and requests a token for a scope; a `200`
//! proves `get-identity` and `get-token` crossed the boundary and returned the
//! stub's fixed token without trapping.
//!
//! The guest is built automatically on first [`find_guest`] call; the test
//! skips locally when it is absent and fails under CI so the pipeline never
//! passes vacuously.

#![cfg(not(target_arch = "wasm32"))]

use std::sync::Arc;

use anyhow::{Context as _, Result};
use omnia::wasmtime_wasi::ResourceTable;
use omnia::{Backend as _, DeploymentBuilder, HasHttp, MountRegistry, Runtime, StoreCtx};
use omnia_testkit::{find_guest, http};
use omnia_wasi_http::{HttpDefault, WasiHttp, WasiHttpCtxView};
use omnia_wasi_identity::{HasIdentity, IdentityStub, WasiIdentity, WasiIdentityCtx};
use omnia_wasi_otel::{HasOtel, OtelDefault, WasiOtel, WasiOtelCtx};

/// The `examples/identity` backend bundle with the stub in place of the
/// OAuth-backed default, so the seam runs without credentials.
#[derive(Clone)]
struct Bundle {
    http: HttpDefault,
    otel: OtelDefault,
    identity: IdentityStub,
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

impl HasIdentity for Bundle {
    fn identity_ctx(&mut self) -> &mut dyn WasiIdentityCtx {
        &mut self.identity
    }
}

async fn runtime() -> Result<Option<Runtime<Bundle>>> {
    let Some(wasm) = find_guest("identity_wasm.wasm") else {
        return Ok(None);
    };

    let bundle = Bundle {
        http: HttpDefault::connect().await.context("connecting http")?,
        otel: OtelDefault::connect().await.context("connecting otel")?,
        identity: IdentityStub::connect().await.context("connecting identity stub")?,
    };

    let mut deployment =
        DeploymentBuilder::new().wasm(wasm).build::<StoreCtx<Bundle>>().await.context("build")?;
    deployment.host::<WasiHttp, Bundle>().context("link http")?;
    deployment.host::<WasiOtel, Bundle>().context("link otel")?;
    deployment.host::<WasiIdentity, Bundle>().context("link identity")?;
    let registry = deployment.into_registry().context("assemble registry")?;

    Ok(Some(Runtime::from_parts(
        Arc::new(registry),
        Vec::new(),
        Arc::new(MountRegistry::default()),
        bundle,
    )))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn get_identity_then_token() -> Result<()> {
    let Some(runtime) = runtime().await? else {
        return Ok(());
    };

    let response = http::get(&runtime, "/").await?;
    assert!(
        response.status().is_success(),
        "guest resolves an identity and obtains a token across the boundary: {:?}",
        response.body()
    );

    Ok(())
}
