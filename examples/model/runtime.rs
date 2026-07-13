//! Model example runtime.
//!
//! Registers the `WasiModel` host backed by an example-local replay backend:
//! the checked-in fixture is embedded at compile time and served through the
//! testkit `ReplayBackend`, so the run is deterministic with no live model, no
//! network, and no configuration. Command mode drives the `create` guest's
//! `wasi:cli/run` export once; the end-to-end completion is also exercised by
//! the seam suite (`crates/seam-suite/tests/seam/model.rs`). See `README.md`.

cfg_if::cfg_if! {
    if #[cfg(not(target_arch = "wasm32"))] {
        use std::sync::Arc;

        use omnia_testkit::model::{Fixture, ReplayBackend};
        use omnia_wasi_model::{Answer, FutureResult, Request, ToolHost, WasiModel, WasiModelCtx};
        use omnia_wasi_otel::{WasiOtel, OtelDefault};

        // The checked-in replay fixture, embedded so the run needs no
        // fixture-directory configuration.
        const FIXTURE: &str = include_str!("fixtures/4855dccaa38b7e6d.json");

        #[derive(Clone, Debug)]
        struct EmbeddedReplay(ReplayBackend);

        #[derive(Clone, Copy, Debug)]
        struct NoOptions;

        impl omnia::FromEnv for NoOptions {
            fn from_env() -> anyhow::Result<Self> {
                Ok(Self)
            }
        }

        impl omnia::Backend for EmbeddedReplay {
            type ConnectOptions = NoOptions;

            async fn connect_with(_options: NoOptions) -> anyhow::Result<Self> {
                let fixture: Fixture = serde_json::from_str(FIXTURE)?;
                Ok(Self(ReplayBackend::new([fixture])?))
            }
        }

        impl WasiModelCtx for EmbeddedReplay {
            fn complete(
                &self, request: Request, tool_host: Arc<dyn ToolHost>,
            ) -> FutureResult<Answer> {
                self.0.complete(request, tool_host)
            }
        }

        omnia::runtime!({
            mode: command,
            hosts: {
                WasiOtel: OtelDefault,
                WasiModel: EmbeddedReplay,
            }
        });
    } else {
        fn main() {}
    }
}
