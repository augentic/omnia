//! Seam test for the MCP transport: the `mcp` example guest serves
//! `omnia_guest::mcp::router` (a guest-side library, no host interface of its
//! own) over `wasi:http`, so its seam is JSON-RPC crossing the HTTP boundary.
//!
//! Driving a `tools/list` and a `tools/call` proves the streamable-HTTP router,
//! compiled into a guest, parses a JSON-RPC request, dispatches to the guest's
//! `McpServer`, and returns a well-formed response — the transport contract the
//! guest-side protocol unit tests cannot cover because they never cross a wasm
//! boundary.
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

/// The `examples/mcp` backend bundle: `wasi:http` + `wasi:otel`.
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
    let Some(wasm) = find_guest("mcp_wasm.wasm", "cargo make build-guests") else {
        return Ok(None);
    };

    let bundle = Bundle {
        http: HttpDefault::connect().await.context("connecting http")?,
        otel: OtelDefault::connect().await.context("connecting otel")?,
    };

    let mut deployment =
        DeploymentBuilder::new().wasm(wasm).build::<StoreCtx<Bundle>>().await.context("build")?;
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
async fn jsonrpc() -> Result<()> {
    let Some(runtime) = runtime().await? else {
        return Ok(());
    };

    // tools/list advertises the guest's two document tools.
    let listed =
        http::post_json(&runtime, "/mcp/docs", r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#)
            .await?;
    assert!(listed.status().is_success(), "tools/list crosses the http boundary");
    let listed: serde_json::Value = serde_json::from_slice(listed.body())?;
    let names: Vec<&str> = listed["result"]["tools"]
        .as_array()
        .context("tools/list returns a tools array")?
        .iter()
        .filter_map(|tool| tool["name"].as_str())
        .collect();
    assert!(names.contains(&"list_docs"), "advertises list_docs: {names:?}");
    assert!(names.contains(&"read_doc"), "advertises read_doc: {names:?}");

    // tools/call reaches the guest's McpServer and returns the read document.
    let called = http::post_json(
        &runtime,
        "/mcp/docs",
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"read_doc","arguments":{"name":"overview"}}}"#,
    )
    .await?;
    assert!(called.status().is_success(), "tools/call crosses the http boundary");
    let called: serde_json::Value = serde_json::from_slice(called.body())?;
    let text = called["result"]["content"][0]["text"].as_str().unwrap_or_default();
    assert!(text.contains("Widget Service Overview"), "returns the overview document: {text:?}");

    Ok(())
}
