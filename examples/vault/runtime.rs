//! Vault example runtime.

cfg_select! {
    not(target_arch = "wasm32") => {
        use omnia_wasi_http::{HttpDefault, WasiHttp};
        use omnia_wasi_otel::{OtelDefault, WasiOtel};
        use omnia_wasi_vault::{VaultDefault, WasiVault};

        omnia::runtime!({
            hosts: {
                WasiHttp: HttpDefault,
                WasiOtel: OtelDefault,
                WasiVault: VaultDefault,
            }
        });
    }
    _ => {
        fn main() {}
    }
}
