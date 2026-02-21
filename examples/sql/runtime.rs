//! SQL example runtime.

cfg_if::cfg_if! {
    if #[cfg(not(target_arch = "wasm32"))] {
        use omnia_wasi_http::{WasiHttp, HttpDefault};
        use omnia_wasi_otel::{WasiOtel, OtelDefault};
        use omnia_wasi_sql::{WasiSql, SqlDefault};

        omnia::runtime!({
            main: true,
            hosts: {
                WasiHttp: HttpDefault,
                WasiOtel: OtelDefault,
                WasiSql: SqlDefault,
            }
        });
    } else {
        fn main() {}
    }
}
