//! Deployment-supplied HTTP listener seam: the trigger server adopts the
//! pre-bound listener (serving on its exact address, no `HTTP_ADDR` bind),
//! and every guest store sees `HTTP_ADDR` injected with the listener's local
//! address — overriding anything inherited from the host environment.

use std::time::Duration;

use anyhow::{Context as _, Result};
use omnia::wasmtime_wasi::ResourceTable;
use omnia::{Backends, DeploymentBuilder, HasHttp, Manifest, Runtime, StoreCtx};
use omnia_testkit::{find_guest, temp_manifest};
use omnia_wasi_http::{HttpDefault, WasiHttp, WasiHttpCtxView};
use omnia_wasi_otel::{HasOtel, OtelDefault, WasiOtel, WasiOtelCtx};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

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

// The server adopts the supplied listener — the request lands on its exact
// pre-bound address — and the guest sees the injected `HTTP_ADDR` carrying
// that same address (override semantics: the entry wins over anything the
// host process inherited).
#[test]
fn supplied_listener() -> Result<()> {
    fixture::RT.block_on(async {
        let guest_a = find_guest("http_routing_a_wasm.wasm");
        let manifest = temp_manifest(&format!(
            "[[guest]]\n\
             id = \"a\"\n\
             source.path = \"{a}\"\n\n\
             [[route.http]]\n\
             prefix = \"/a\"\n\
             guest = \"a\"\n",
            a = guest_a.display(),
        ))?;

        let listener =
            std::net::TcpListener::bind("127.0.0.1:0").context("pre-binding the listener")?;
        let addr = listener.local_addr().context("reading the pre-bound address")?;

        let builder = DeploymentBuilder::new()
            .manifest(Manifest::from_config(manifest.path())?)
            .http_listener(listener)
            .precompiled();
        // SAFETY: `find_guest` only returns artifacts this workspace built
        // and serialized itself (`cargo make test-guests`).
        let deployment = unsafe { builder.build::<StoreCtx<Bundle>>() }.await.context("build")?;
        let runtime = Runtime::<Bundle>::new(deployment, |deployment| {
            deployment.host::<WasiHttp, Bundle>()?;
            deployment.host::<WasiOtel, Bundle>()?;
            Ok(())
        })
        .await
        .context("assembling runtime")?;

        let state = runtime.clone();
        let served = tokio::spawn(async move { omnia::Server::run(&WasiHttp, &state).await });

        // The listener is already bound, so the connection queues in the OS
        // backlog even if the accept loop has not started yet.
        let response = tokio::time::timeout(Duration::from_secs(30), async {
            let mut stream = tokio::net::TcpStream::connect(addr)
                .await
                .context("connecting to the pre-bound address")?;
            stream
                .write_all(b"GET /a HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
                .await
                .context("writing request")?;
            let mut response = Vec::new();
            stream.read_to_end(&mut response).await.context("reading response")?;
            anyhow::Ok(String::from_utf8_lossy(&response).into_owned())
        })
        .await
        .context("request timed out")??;

        assert!(response.starts_with("HTTP/1.1 200"), "{response}");
        assert!(response.contains("guest a"), "{response}");
        assert!(
            response.contains(&format!("HTTP_ADDR={addr}")),
            "the guest sees the injected listener address: {response}"
        );

        served.abort();
        Ok(())
    })
}
