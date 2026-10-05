//! SQL example runtime.

cfg_select! {
    not(target_arch = "wasm32") => {
        use omnia_wasi_http::{HttpDefault, WasiHttp};
        use omnia_wasi_otel::{OtelDefault, WasiOtel};
        use omnia_wasi_sql::{SqlDefault, WasiSql};

        omnia::runtime!({
            hosts: {
                WasiHttp: HttpDefault,
                WasiOtel: OtelDefault,
                WasiSql: SqlDefault,
            }
        });
    }
    _ => {
        fn main() {}
    }
}
