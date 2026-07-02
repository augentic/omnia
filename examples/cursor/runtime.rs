//! Cursor example runtime.
//!
//! Binds `WasiModel` to the spawned-`cursor-agent` backend and serves two wasm
//! guests over one HTTP trigger: `/ask` (calls `complete`) and `/mcp/docs` (the
//! read-only MCP documentation server the agent reads). See `README.md`.

cfg_if::cfg_if! {
    if #[cfg(not(target_arch = "wasm32"))] {
        use omnia_cursor::Client;
        use omnia_wasi_http::{HttpDefault, WasiHttp};
        use omnia_wasi_model::WasiModel;
        use omnia_wasi_otel::{OtelDefault, WasiOtel};

        omnia::runtime!({
            hosts: {
                WasiHttp: HttpDefault,
                WasiOtel: OtelDefault,
                WasiModel: Client,
            }
        });
    } else {
        fn main() {}
    }
}
