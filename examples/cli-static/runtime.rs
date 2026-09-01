//! Direct-command deployment, entirely macro-expressed.
//!
//! The deployment lands in one [`omnia::runtime!`] invocation: a static guest
//! from the inline manifest. The static guest is the sole `wasi:cli/run`
//! exporter, so command mode routes to it with no configuration. Because the
//! deployment is compiled in, command mode makes the binary a direct command
//! — no host `run` grammar, the binary's argv belongs to the guest, so it
//! runs as `cli-static greet Ada`, not
//! `cli-static run -- greet Ada`; see `README.md`.

cfg_if::cfg_if! {
    if #[cfg(not(target_arch = "wasm32"))] {
        use omnia_wasi_otel::{WasiOtel, OtelDefault};

        omnia::runtime!({
            mode: command,
            hosts: {
                WasiOtel: OtelDefault,
            },
            guests: [
                { id: "cli", source: concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../target/wasm32-wasip2/debug/examples/cli_wasm.wasm",
                ) },
            ],
        });
    } else {
        fn main() {}
    }
}
