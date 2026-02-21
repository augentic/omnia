//! Config example runtime.

cfg_if::cfg_if! {
    if #[cfg(not(target_arch = "wasm32"))] {
        use omnia_wasi_config::{WasiConfig, ConfigDefault};
        use omnia_wasi_http::{WasiHttp, HttpDefault};
        use omnia_wasi_otel::{WasiOtel, OtelDefault};

        omnia::runtime!({
            main: true,
            hosts: {
                WasiConfig: ConfigDefault,
                WasiHttp: HttpDefault,
                WasiOtel: OtelDefault,
            }
        });
    } else {
        fn main() {}
    }
}
