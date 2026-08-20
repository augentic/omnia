//! Model example runtime.
//!
//! Registers the `WasiModel` host backed by the testkit `Scripted` double
//! serving a fixed schema answer, so the run is deterministic with no live
//! model, no network, and no configuration. Command mode drives the `create`
//! guest's `wasi:cli/run` export once; the end-to-end completion is also
//! exercised by the seam suite (`crates/seam-suite/tests/seam/model.rs`).
//! See `README.md`.

cfg_if::cfg_if! {
    if #[cfg(not(target_arch = "wasm32"))] {
        use std::sync::Arc;

        use omnia_testkit::model::Scripted;
        use omnia_wasi_model::{Answer, FutureResult, Request, ToolHost, WasiModel, WasiModelCtx};
        use omnia_wasi_otel::{WasiOtel, OtelDefault};

        #[derive(Clone, Debug)]
        struct CannedVerdict(Scripted);

        impl omnia::Backend for CannedVerdict {
            type ConnectOptions = omnia::NoOptions;

            fn connect_with(
                _options: omnia::NoOptions,
            ) -> impl std::future::Future<Output = anyhow::Result<Self>> {
                std::future::ready(Ok(Self(Scripted::json(serde_json::json!({
                    "verdict": "pass",
                    "reason": "the bounds check is correct",
                })))))
            }
        }

        impl WasiModelCtx for CannedVerdict {
            fn complete(
                &self, request: Request, tool_host: Arc<dyn ToolHost>,
            ) -> FutureResult<Answer> {
                self.0.complete(request, tool_host)
            }
        }

        omnia::runtime!({
            mode: command,
            config: concat!(env!("CARGO_MANIFEST_DIR"), "/model/omnia.toml"),
            hosts: {
                WasiOtel: OtelDefault,
                WasiModel: CannedVerdict,
            }
        });
    } else {
        fn main() {}
    }
}
