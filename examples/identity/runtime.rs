//! Identity example runtime.

cfg_if::cfg_if! {
    if #[cfg(not(target_arch = "wasm32"))] {
        use qwasr_wasi_http::{WasiHttp, HttpDefault};
        use qwasr_wasi_identity::{WasiIdentity, IdentityDefault};
        use qwasr_wasi_otel::{WasiOtel, OtelDefault};

        qwasr::runtime!({
            main: true,
            hosts: {
                WasiHttp: HttpDefault,
                WasiOtel: OtelDefault,
                WasiIdentity: IdentityDefault,
            }
        });
    } else {
        fn main() {}
    }
}
