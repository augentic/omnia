//! Multi-guest HTTP routing seam: one server fronts two guests, and
//! `[[route.http]]` prefixes select the guest per request by longest match.

use std::sync::Arc;

use anyhow::{Context as _, Result};
use omnia::wasmtime_wasi::ResourceTable;
use omnia::{Backend as _, DeploymentBuilder, HasHttp, Manifest, MountRegistry, Runtime, StoreCtx};
use omnia_testkit::{find_guest, http, temp_manifest};
use omnia_wasi_http::{HttpDefault, WasiHttp, WasiHttpCtxView};
use omnia_wasi_otel::{HasOtel, OtelDefault, WasiOtel, WasiOtelCtx};

use crate::fixture;

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

async fn runtime() -> Result<Runtime<Bundle>> {
    let guest_a = find_guest("http_routing_a_wasm.wasm");
    let guest_b = find_guest("http_routing_b_wasm.wasm");

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

    let builder =
        DeploymentBuilder::new().manifest(Manifest::from_config(manifest.path())?).precompiled();
    // SAFETY: `find_guest` only returns artifacts this workspace built and
    // serialized itself (`cargo make test-guests`).
    let mut deployment = unsafe { builder.build::<StoreCtx<Bundle>>() }.await.context("build")?;
    deployment.host::<WasiHttp, Bundle>().context("link http")?;
    deployment.host::<WasiOtel, Bundle>().context("link otel")?;
    let registry = deployment.into_registry().context("assemble registry")?;

    Ok(Runtime::from_parts(
        Arc::new(registry),
        Vec::new(),
        Arc::new(MountRegistry::default()),
        bundle,
    ))
}

#[test]
fn prefix_routing() -> Result<()> {
    fixture::RT.block_on(async {
        let runtime = runtime().await?;

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
    })
}
