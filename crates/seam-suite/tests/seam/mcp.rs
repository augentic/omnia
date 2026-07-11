//! MCP transport seam: the `mcp` example guest serves `omnia_guest::mcp::router`
//! (a guest-side library, no host interface of its own) over `wasi:http`, so
//! its seam is JSON-RPC crossing the HTTP boundary.

use anyhow::{Context as _, Result};
use omnia::wasmtime_wasi::ResourceTable;
use omnia::{Backend as _, HasHttp, Runtime};
use omnia_testkit::{http, single_guest};
use omnia_wasi_http::{HttpDefault, WasiHttp, WasiHttpCtxView};
use omnia_wasi_otel::{HasOtel, OtelDefault, WasiOtel, WasiOtelCtx};

use crate::fixture;

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

async fn runtime() -> Result<Runtime<Bundle>> {
    let bundle = Bundle {
        http: HttpDefault::connect().await.context("connecting http")?,
        otel: OtelDefault::connect().await.context("connecting otel")?,
    };

    let guest = single_guest("mcp_wasm.wasm", bundle).await?;
    guest.host::<WasiHttp>()?.host::<WasiOtel>()?.into_runtime()
}

#[test]
fn jsonrpc() -> Result<()> {
    fixture::RT.block_on(async {
        let runtime = runtime().await?;

        // tools/list advertises the guest's two document tools.
        let listed = http::post_json(
            &runtime,
            "/mcp/docs",
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#,
        )
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
        assert!(
            text.contains("Widget Service Overview"),
            "returns the overview document: {text:?}"
        );

        Ok(())
    })
}
