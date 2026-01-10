cfg_if::cfg_if! {
    if #[cfg(not(target_arch = "wasm32"))] {
        use yetti_wasi_http::{WasiHttp, HttpDefault};
        use yetti_wasi_keyvalue::{WasiKeyValue, KeyValueDefault};
        use yetti_wasi_otel::{WasiOtel, OtelDefault};

        yetti::runtime!({
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
