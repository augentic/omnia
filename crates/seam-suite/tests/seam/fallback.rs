//! HTTP trigger fallback seam: a fully dynamic deployment starts with zero
//! guests, and unrouted request paths are mapped to guest identities that a
//! [`GuestResolver`] faults in on first use (RFC guest-resolution §4.5).
//!
//! Driven through [`omnia_testkit::http::HttpHarness`] so routing is
//! snapshotted once across requests — the production server's boot-frozen
//! router lifetime. The two `examples/http-routing` guests give each faulted
//! tenant a distinct response, proving the boot-frozen router never flips to
//! catch-all for a late guest.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use anyhow::{Context as _, Result, ensure};
use futures::FutureExt as _;
use omnia::wasmtime_wasi::ResourceTable;
use omnia::{
    Backends, DeploymentBuilder, FutureResult, GuestArtifact, GuestId, GuestResolver, HasHttp,
    Runtime, StoreCtx,
};
use omnia_testkit::find_guest;
use omnia_testkit::http::HttpHarness;
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

impl Backends for Bundle {
    async fn connect() -> Result<Self> {
        Ok(Self {
            http: <HttpDefault as omnia::Backend>::connect().await.context("connecting http")?,
            otel: <OtelDefault as omnia::Backend>::connect().await.context("connecting otel")?,
        })
    }
}

/// Locate the serialized `.bin` for `file` as a registration artifact.
fn precompiled(file: &str) -> Result<GuestArtifact> {
    let path = find_guest(file);
    ensure!(
        path.extension().is_some_and(|ext| ext == "bin"),
        "{} has no serialized .bin sibling; run `cargo make test-guests`",
        path.display()
    );
    let bytes =
        std::fs::read(&path).with_context(|| format!("reading guest {}", path.display()))?;
    // SAFETY: the artifact was built and serialized by this workspace's own
    // `cargo make test-guests` pipeline (omnia's compile path).
    Ok(unsafe { GuestArtifact::precompiled(bytes) })
}

/// A counting tenant resolver: `a` and `b` map to the two routing guests,
/// `bad` to a component with no http handler, everything else to a
/// definitive miss.
struct TenantResolver {
    calls: AtomicUsize,
}

impl GuestResolver for TenantResolver {
    fn resolve(
        &self, guest: GuestId, _expected_export: String,
    ) -> FutureResult<Option<GuestArtifact>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let outcome = match guest.to_string().as_str() {
            "a" => precompiled("http_routing_a_wasm.wasm").map(Some),
            "b" => precompiled("http_routing_b_wasm.wasm").map(Some),
            "bad" => precompiled("guest_link_extra_wasm.wasm").map(Some),
            _ => Ok(None),
        };
        async move { outcome }.boxed()
    }
}

/// Build the empty dynamic deployment through the builder-carried hooks (the
/// `Runtime::new` install path) and snapshot one harness over it.
async fn harness() -> Result<(HttpHarness<Bundle>, Runtime<Bundle>, Arc<TenantResolver>)> {
    let resolver = Arc::new(TenantResolver {
        calls: AtomicUsize::new(0),
    });

    let deployment = DeploymentBuilder::new()
        .dynamic()
        .resolver(Arc::clone(&resolver) as Arc<dyn GuestResolver>)
        .http_fallback(|path: &str| match path {
            "/a" => Some(GuestId::from("a")),
            "/b" => Some(GuestId::from("b")),
            "/bad" => Some(GuestId::from("bad")),
            "/missing" => Some(GuestId::from("missing")),
            _ => None,
        })
        .build::<StoreCtx<Bundle>>()
        .await
        .context("building empty dynamic deployment")?;

    let runtime = Runtime::<Bundle>::new(deployment, |deployment| {
        deployment.host::<WasiHttp, Bundle>()?;
        deployment.host::<WasiOtel, Bundle>()?;
        Ok(())
    })
    .await
    .context("assembling runtime")?;

    let harness = HttpHarness::new(runtime.clone()).context("snapshotting http routing")?;
    Ok((harness, runtime, resolver))
}

// Two tenants faulted in sequentially through one harness each reach their
// own guest: the boot-frozen router never flips to catch-all for a late
// guest, and a registry hit never re-resolves.
#[test]
fn fallback_faults_tenants_in() -> Result<()> {
    fixture::RT.block_on(async {
        let (harness, runtime, resolver) = harness().await?;

        let a = harness.get("/a").await?;
        assert!(a.status().is_success(), "/a faults tenant a in");
        assert!(String::from_utf8_lossy(a.body()).contains("guest a"), "{:?}", a.body());

        let b = harness.get("/b").await?;
        assert!(b.status().is_success(), "/b faults tenant b in");
        assert!(String::from_utf8_lossy(b.body()).contains("guest b"), "{:?}", b.body());

        assert!(runtime.registry().get(&GuestId::from("a")).is_some(), "tenant a is registered");
        assert!(runtime.registry().get(&GuestId::from("b")).is_some(), "tenant b is registered");
        assert_eq!(resolver.calls.load(Ordering::SeqCst), 2, "one resolve per tenant");

        // A registered tenant dispatches off the registry, not the resolver.
        let again = harness.get("/a").await?;
        assert!(String::from_utf8_lossy(again.body()).contains("guest a"), "{:?}", again.body());
        assert_eq!(resolver.calls.load(Ordering::SeqCst), 2, "a registry hit never resolves");

        Ok(())
    })
}

// A path the fallback declines stays unrouted, and a fallback identity the
// resolver has no component for fails as unregistered — neither is cached.
#[test]
fn fallback_negative_outcomes() -> Result<()> {
    fixture::RT.block_on(async {
        let (harness, runtime, resolver) = harness().await?;

        let error = harness.get("/nope").await.expect_err("a declined path is unrouted");
        assert!(format!("{error:#}").contains("no route matched path"), "{error:#}");
        assert_eq!(resolver.calls.load(Ordering::SeqCst), 0, "no fallback id, no resolve");

        let error = harness.get("/missing").await.expect_err("an unknown tenant fails");
        assert!(format!("{error:#}").contains("is not registered"), "{error:#}");
        assert!(
            runtime.registry().get(&GuestId::from("missing")).is_none(),
            "a definitive miss registers nothing"
        );
        assert_eq!(resolver.calls.load(Ordering::SeqCst), 1, "the miss consulted the resolver");

        Ok(())
    })
}

// A fallback guest lacking `wasi:http/incoming-handler` is refused at
// resolve-time registration — an error, not a partial route.
#[test]
fn fallback_guest_without_handler_refused() -> Result<()> {
    fixture::RT.block_on(async {
        let (harness, runtime, _resolver) = harness().await?;

        let error = harness.get("/bad").await.expect_err("a handler-less guest is refused");
        assert!(
            format!("{error:#}").contains("does not export interface `wasi:http/handler`"),
            "{error:#}"
        );
        assert!(
            runtime.registry().get(&GuestId::from("bad")).is_none(),
            "a refused artifact must not publish the guest"
        );

        Ok(())
    })
}
