//! HTTP proxy example runtime.

cfg_if::cfg_if! {
    if #[cfg(not(target_arch = "wasm32"))] {
        use omnia_wasi_http::{WasiHttp, HttpDefault};
        use omnia_wasi_keyvalue::{WasiKeyValue, KeyValueDefault};
        use omnia_wasi_otel::{WasiOtel, OtelDefault};

        omnia::runtime!({
            main: true,
            hosts: {
                WasiHttp: HttpDefault,
                WasiKeyValue: KeyValueDefault,
                WasiOtel: OtelDefault,
            }
        });
    } else {
        fn main() {}
    }
}
