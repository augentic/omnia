//! # DocStore Runtime (Default Backend)
//!
//! Host binary for the `wasi:docstore` example. Uses the in-memory default backend.

cfg_select! {
    not(target_arch = "wasm32") => {
        use omnia_wasi_docstore::{DocStoreDefault, WasiDocStore};
        use omnia_wasi_http::{HttpDefault, WasiHttp};
        use omnia_wasi_otel::{OtelDefault, WasiOtel};

        omnia::runtime!({
            hosts: {
                WasiHttp: HttpDefault,
                WasiOtel: OtelDefault,
                WasiDocStore: DocStoreDefault,
            }
        });
    }
    _ => {
        fn main() {}
    }
}
