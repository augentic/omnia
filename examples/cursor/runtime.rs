//! Cursor example runtime.
//!
//! Command mode drives the `ask` guest's `wasi:cli/run` export once and exits
//! with its status while the HTTP trigger keeps serving `/mcp/docs` in the
//! background for the spawned `cursor-agent`. See `README.md`.

cfg_if::cfg_if! {
    if #[cfg(not(target_arch = "wasm32"))] {
        use anyhow::{Context, Result};
        use omnia::futures::future;
        use omnia::{main as omnia_main, Backend, FromEnv, Mode, Runtime, RuntimeHooks, Server};
        use omnia_cursor::{Client, ConnectOptions};
        use omnia_wasi_http::{HttpDefault, WasiHttp};
        use omnia_wasi_model::WasiModel;
        use omnia_wasi_otel::{OtelDefault, WasiOtel};
        use std::process::ExitCode;

        // Matches the `docs` MCP grant in `guest.rs` and the `/mcp/docs` route
        // in `omnia.toml`. Override with `CURSOR_MCP_SERVERS` when needed.
        const DEFAULT_MCP_SERVERS: &str = r#"{"docs":"http://localhost:8080/mcp/docs"}"#;

        fn cursor_connect_options() -> Result<ConnectOptions> {
            let mut options = <ConnectOptions as FromEnv>::from_env()?;
            if options.mcp_servers.is_none() {
                options.mcp_servers = Some(DEFAULT_MCP_SERVERS.to_owned());
            }
            if options.workspace.is_none() {
                let dir = std::env::temp_dir().join(format!(
                    "omnia-cursor-workspace-{}",
                    std::process::id()
                ));
                std::fs::create_dir_all(&dir)
                    .with_context(|| format!("creating workspace {}", dir.display()))?;
                options.workspace = Some(dir.to_string_lossy().into_owned());
            }
            Ok(options)
        }

        #[derive(Clone)]
        struct Backends {
            http_default: HttpDefault,
            otel_default: OtelDefault,
            client: Client,
        }

        impl omnia::Backends for Backends {
            async fn connect() -> Result<Self> {
                let (http_default, otel_default, client) = tokio::try_join!(
                    HttpDefault::connect(),
                    OtelDefault::connect(),
                    Client::connect_with(cursor_connect_options()?),
                )?;
                Ok(Self { http_default, otel_default, client })
            }
        }

        impl omnia::HasHttp for Backends {
            fn http_view<'a>(
                &'a mut self,
                table: &'a mut omnia::wasmtime_wasi::ResourceTable,
            ) -> omnia_wasi_http::WasiHttpCtxView<'a> {
                self.http_default.as_view(table)
            }
        }

        impl omnia_wasi_otel::HasOtel for Backends {
            fn otel_ctx(&mut self) -> &mut dyn omnia_wasi_otel::WasiOtelCtx {
                &mut self.otel_default
            }
        }

        impl omnia_wasi_model::HasModel for Backends {
            fn model_ctx(&mut self) -> &mut dyn omnia_wasi_model::WasiModelCtx {
                &mut self.client
            }
        }

        struct Hooks;

        impl RuntimeHooks<Backends> for Hooks {
            fn link(deployment: &mut omnia::Deployment<omnia::StoreCtx<Backends>>) -> Result<()> {
                deployment.host::<WasiHttp, Backends>()?;
                deployment.host::<WasiOtel, Backends>()?;
                deployment.host::<WasiModel, Backends>()?;
                Ok(())
            }

            async fn serve(runtime: &Runtime<Backends>) -> Result<()> {
                let servers: Vec<future::BoxFuture<'_, Result<()>>> =
                    vec![Box::pin(WasiHttp.run(runtime))];
                future::try_join_all(servers).await?;
                Ok(())
            }
        }

        #[tokio::main]
        pub async fn main() -> ExitCode {
            omnia_main::<Backends, Hooks>(Mode::Command).await
        }
    } else {
        fn main() {}
    }
}
