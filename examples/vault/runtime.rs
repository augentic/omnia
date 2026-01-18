//! Vault example runtime.

cfg_if::cfg_if! {
    if #[cfg(not(target_arch = "wasm32"))] {
        use qwasr_wasi_http::{WasiHttp, HttpDefault};
        use qwasr_wasi_otel::{WasiOtel, OtelDefault};
        use qwasr_wasi_vault::{WasiVault, VaultDefault};

        qwasr::runtime!({
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
