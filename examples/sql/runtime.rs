cfg_if::cfg_if! {
    if #[cfg(not(target_arch = "wasm32"))] {
        use yetti_wasi_http::{WasiHttp, HttpDefault};
        use yetti_wasi_otel::{WasiOtel, OtelDefault};
        use yetti_wasi_sql::{WasiSql, SqlDefault};

        yetti::runtime!({
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
