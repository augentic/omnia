//! Seam test for `wasi:vault`: drive the `vault` example guest over the real
//! `wasi:http` boundary and confirm the secret landed host-side.
//!
//! The guest opens a locker, `set`s the request body under `secret-id`, reads
//! it back, and echoes the parsed JSON. Reading the shared backend afterwards
//! proves the write crossed the WIT boundary into the host vault.
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
use omnia_wasi_otel::{HasOtel, OtelDefault, WasiOtel, WasiOtelCtx};
use omnia_wasi_vault::{HasVault, VaultDefault, WasiVault, WasiVaultCtx};

/// The `examples/vault` backend bundle: `wasi:http` + `wasi:otel` +
/// `wasi:vault`.
#[derive(Clone)]
struct Bundle {
    http: HttpDefault,
    otel: OtelDefault,
    vault: VaultDefault,
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

impl HasVault for Bundle {
    fn vault_ctx(&mut self) -> &mut dyn WasiVaultCtx {
        &mut self.vault
    }
}

/// Build the runtime, returning it plus a handle to the shared vault backend
/// (its `Arc`-backed store is shared across clones, so the handle observes the
/// guest's writes).
async fn runtime() -> Result<Option<(Runtime<Bundle>, VaultDefault)>> {
    let Some(wasm) = find_guest("vault_wasm.wasm", "cargo make build-guests") else {
        return Ok(None);
    };

    let bundle = Bundle {
        http: HttpDefault::connect().await.context("connecting http")?,
        otel: OtelDefault::connect().await.context("connecting otel")?,
        vault: VaultDefault::connect().await.context("connecting vault")?,
    };
    let store_probe = bundle.vault.clone();

    let mut deployment =
        DeploymentBuilder::new().wasm(wasm).build::<StoreCtx<Bundle>>().await.context("build")?;
    deployment.host::<WasiHttp, Bundle>().context("link http")?;
    deployment.host::<WasiOtel, Bundle>().context("link otel")?;
    deployment.host::<WasiVault, Bundle>().context("link vault")?;
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
    let Some((runtime, vault)) = runtime().await? else {
        return Ok(());
    };

    let response = http::post(&runtime, "/", r#"{"token":"s3cret"}"#).await?;
    assert!(response.status().is_success(), "guest completes the vault round-trip");

    // The guest stored the body under `secret-id` in `omnia-locker`; the shared
    // backend must now hold that write.
    let locker = vault.open_locker("omnia-locker".to_owned()).await.context("open locker")?;
    let secret = locker.get("secret-id".to_owned()).await.context("read secret")?;
    assert_eq!(
        secret.as_deref(),
        Some(br#"{"token":"s3cret"}"#.as_slice()),
        "the secret reached the host vault"
    );

    Ok(())
}
