//! Direct-command deployment, entirely macro-expressed.
//!
//! The deployment lands in one [`omnia::runtime!`] invocation: a guest
//! embedded from the inline manifest. `build.rs` compiles the guest and
//! names its artifact in `CLI_WASM`; the macro embeds those bytes with
//! `include_bytes!` and names the guest by the file's stem, `cli_wasm`. The
//! guest is the sole `wasi:cli/run` exporter, so command mode routes to it
//! with no configuration. Because the deployment is compiled in, command
//! mode makes the binary a direct command — no host `run` grammar, the
//! binary's argv belongs to the guest, so it runs as `cli-static greet Ada`,
//! not `cli-static run -- greet Ada`; see `README.md`.

cfg_if::cfg_if! {
    if #[cfg(not(target_arch = "wasm32"))] {
        use omnia_wasi_otel::{WasiOtel, OtelDefault};

        omnia::runtime!({
            mode: command,
            hosts: {
                WasiOtel: OtelDefault,
            },
            guests: [
                { path: env!("CLI_WASM") },
            ],
        });
    } else {
        fn main() {}
    }
}
