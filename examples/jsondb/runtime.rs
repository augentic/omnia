//! # JsonDb Runtime (Default Backend)
//!
//! Host binary for the `wasi:jsondb` example. Uses PoloDB as the default backend.

cfg_if::cfg_if! {
    if #[cfg(not(target_arch = "wasm32"))] {
        use omnia_wasi_http::{WasiHttp, HttpDefault};
        use omnia_wasi_jsondb::{WasiJsonDb, JsonDbDefault};
        use omnia_wasi_otel::{WasiOtel, OtelDefault};

        omnia::runtime!({
            main: true,
            hosts: {
                WasiHttp: HttpDefault,
                WasiOtel: OtelDefault,
                WasiJsonDb: JsonDbDefault,
            }
        });
    } else {
        fn main() {}
    }
}
