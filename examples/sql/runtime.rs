//! SQL example runtime.

cfg_if::cfg_if! {
    if #[cfg(not(target_arch = "wasm32"))] {
        use qwasr_wasi_http::{WasiHttp, HttpDefault};
        use qwasr_wasi_otel::{WasiOtel, OtelDefault};
        use qwasr_wasi_sql::{WasiSql, SqlDefault};

        qwasr::runtime!({
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
