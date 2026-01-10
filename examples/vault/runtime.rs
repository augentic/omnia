cfg_if::cfg_if! {
    if #[cfg(not(target_arch = "wasm32"))] {
        use yetti_wasi_http::{WasiHttp, HttpDefault};
        use yetti_wasi_otel::{WasiOtel, OtelDefault};
        use yetti_wasi_vault::{WasiVault, VaultDefault};

        yetti::runtime!({
            main: true,
            hosts: {
                WasiHttp: HttpDefault,
                WasiOtel: OtelDefault,
                WasiVault: VaultDefault,
            }
        });
    } else {
        fn main() {}
    }
}
