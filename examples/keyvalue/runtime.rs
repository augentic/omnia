//! Key-value example runtime.

cfg_if::cfg_if! {
    if #[cfg(not(target_arch = "wasm32"))] {
        use qwasr_wasi_http::{WasiHttp, HttpDefault};
        use qwasr_wasi_keyvalue::{WasiKeyValue, KeyValueDefault};
        use qwasr_wasi_otel::{WasiOtel, OtelDefault};

        qwasr::runtime!({
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
