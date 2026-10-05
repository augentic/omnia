//! Identity example runtime.

cfg_select! {
    not(target_arch = "wasm32") => {
        use omnia_wasi_http::{HttpDefault, WasiHttp};
        use omnia_wasi_identity::{IdentityDefault, WasiIdentity};
        use omnia_wasi_otel::{OtelDefault, WasiOtel};

        omnia::runtime!({
            hosts: {
                WasiHttp: HttpDefault,
                WasiOtel: OtelDefault,
                WasiIdentity: IdentityDefault,
            }
        });
    }
    _ => {
        fn main() {}
    }
}
