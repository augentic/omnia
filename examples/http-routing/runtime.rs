//! Multi-guest HTTP routing example runtime.
//!
//! One HTTP server fronts two guests; each guest's `routes.http` prefixes in
//! `omnia.toml` select it per request by longest-prefix match.

cfg_select! {
    not(target_arch = "wasm32") => {
        use omnia_wasi_http::{HttpDefault, WasiHttp};
        use omnia_wasi_otel::{OtelDefault, WasiOtel};

        omnia::runtime!({
            manifest: concat!(env!("CARGO_MANIFEST_DIR"), "/http-routing/omnia.toml"),
            hosts: {
                WasiHttp: HttpDefault,
                WasiOtel: OtelDefault,
            }
        });
    }
    _ => {
        fn main() {}
    }
}
