//! Key-value example runtime.

cfg_if::cfg_if! {
    if #[cfg(not(target_arch = "wasm32"))] {
        use omnia_wasi_http::{WasiHttp, HttpDefault};
        use omnia_wasi_keyvalue::{WasiKeyValue, KeyValueDefault};
        use omnia_wasi_otel::{WasiOtel, OtelDefault};

        omnia::runtime!({
            main: true,
            hosts: {
                WasiHttp: HttpDefault,
                WasiOtel: OtelDefault,
                WasiKeyValue: KeyValueDefault,
            }
        });
    } else {
        fn main() {}
    }
}
