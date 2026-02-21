//! Vault example runtime.

cfg_if::cfg_if! {
    if #[cfg(not(target_arch = "wasm32"))] {
        use omnia_wasi_http::{WasiHttp, HttpDefault};
        use omnia_wasi_otel::{WasiOtel, OtelDefault};
        use omnia_wasi_vault::{WasiVault, VaultDefault};

        omnia::runtime!({
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
