//! Model example runtime.
//!
//! Registers the `WasiModel` host backed by the in-tree `ModelDefault` (replay)
//! backend. Command mode drives the `create` guest's `wasi:cli/run` export once;
//! the end-to-end completion is also exercised by the integration test
//! (`crates/wasi-model/tests/replay.rs`). See `README.md`.

cfg_if::cfg_if! {
    if #[cfg(not(target_arch = "wasm32"))] {
        use omnia_wasi_model::{WasiModel, ModelDefault};
        use omnia_wasi_otel::{WasiOtel, OtelDefault};

        omnia::runtime!({
            mode: command,
            hosts: {
                WasiOtel: OtelDefault,
                WasiModel: ModelDefault,
            }
        });
    } else {
        fn main() {}
    }
}
