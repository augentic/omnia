//! Host-mediated linking with a programmatically assembled deployment manifest.
//!
//! The guests are read into bytes here, so they load at boot as the
//! `runtime!`-embedded pair in `runtime.rs` does and startup itself proves
//! the `echo` import is wired; a `GuestEntry` naming a path instead would
//! load at the guest's first use (see `register.rs`).

cfg_if::cfg_if! {
    if #[cfg(not(target_arch = "wasm32"))] {
        use std::path::Path;

        use anyhow::Context as _;
        use omnia::{DeploymentBuilder, GuestEntry, Manifest};
        use omnia_wasi_http::{HttpDefault, WasiHttp};
        use omnia_wasi_otel::{OtelDefault, WasiOtel};

        mod host {
            use super::*;

            omnia::runtime!({
                hosts: {
                    WasiHttp: HttpDefault,
                    WasiOtel: OtelDefault,
                }
            });
        }

        fn main() -> anyhow::Result<()> {
            let artifacts =
                Path::new(env!("CARGO_MANIFEST_DIR")).join("../target/wasm32-wasip2/debug/examples");
            let read = |file: &str| {
                std::fs::read(artifacts.join(file)).with_context(|| {
                    format!("{file} not built: cargo build -p examples --examples --target wasm32-wasip2")
                })
            };
            let manifest = Manifest::new()
                .guest(GuestEntry::new("responder", read("guest_link_responder_wasm.wasm")?))
                .guest(GuestEntry::new("router", read("guest_link_router_wasm.wasm")?));

            host::run(DeploymentBuilder::new().manifest(manifest))?;
            Ok(())
        }
    } else {
        fn main() {}
    }
}
