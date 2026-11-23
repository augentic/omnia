//! Config example runtime.

cfg_select! {
    not(target_arch = "wasm32") => {
        use omnia_wasi_config::{ConfigDefault, ConnectOptions, WasiConfig};
        use omnia_wasi_http::{HttpDefault, WasiHttp};
        use omnia_wasi_otel::{OtelDefault, WasiOtel};

        omnia::runtime!({
            hosts: {
                WasiConfig: ConfigDefault(ConnectOptions {
                    keys: vec!["HOME".to_owned(), "USER".to_owned()],
                }),
                WasiHttp: HttpDefault,
                WasiOtel: OtelDefault,
            }
        });
    }
    _ => {
        fn main() {}
    }
}
