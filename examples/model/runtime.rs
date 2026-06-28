//! Model example runtime.
//!
//! Registers the `WasiModel` host backed by the in-tree `ModelDefault` (replay)
//! backend. The `complete` guest exports a plain `run` rather than an
//! HTTP/messaging trigger, so the end-to-end completion is driven by the
//! integration test (`crates/wasi-model/tests/replay.rs`); running this binary
//! starts the host with the replay backend reading `OMNIA_REPLAY_DIR`. See
//! `README.md`.

cfg_if::cfg_if! {
    if #[cfg(not(target_arch = "wasm32"))] {
        use omnia_wasi_model::{WasiModel, ModelDefault};
        use omnia_wasi_otel::{WasiOtel, OtelDefault};

        omnia::runtime!({
            hosts: {
                WasiOtel: OtelDefault,
                WasiModel: ModelDefault,
            }
        });
    } else {
        fn main() {}
    }
}
