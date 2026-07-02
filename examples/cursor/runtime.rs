//! Cursor example runtime.
//!
//! Command mode drives the `ask` guest's `wasi:cli/run` export once and exits
//! with its status while the HTTP trigger keeps serving `/mcp/docs` in the
//! background for the spawned `cursor-agent`. See `README.md`.

cfg_if::cfg_if! {
    if #[cfg(not(target_arch = "wasm32"))] {
        use omnia_cursor::Client;
        use omnia_wasi_http::{HttpDefault, WasiHttp};
        use omnia_wasi_model::WasiModel;
        use omnia_wasi_otel::{OtelDefault, WasiOtel};

        omnia::runtime!({
            mode: command,
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
