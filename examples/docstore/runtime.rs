//! # DocStore Runtime (Default Backend)
//!
//! Host binary for the `wasi:docstore` example. Uses PoloDB as the default backend.

cfg_if::cfg_if! {
    if #[cfg(not(target_arch = "wasm32"))] {
        use omnia_wasi_http::{WasiHttp, HttpDefault};
        use omnia_wasi_docstore::{WasiDocStore, DocStoreDefault};
        use omnia_wasi_otel::{WasiOtel, OtelDefault};

        omnia::runtime!({
            hosts: {
                WasiHttp: HttpDefault,
                WasiOtel: OtelDefault,
                WasiDocStore: DocStoreDefault,
            }
        });
    } else {
        fn main() {}
    }
}
