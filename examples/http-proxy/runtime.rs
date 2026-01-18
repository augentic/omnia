//! HTTP proxy example runtime.

cfg_if::cfg_if! {
    if #[cfg(not(target_arch = "wasm32"))] {
        use qwasr_wasi_http::{WasiHttp, HttpDefault};
        use qwasr_wasi_keyvalue::{WasiKeyValue, KeyValueDefault};
        use qwasr_wasi_otel::{WasiOtel, OtelDefault};

        qwasr::runtime!({
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
