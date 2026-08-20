//! Direct-command deployment, entirely macro-expressed.
//!
//! The deployment lands in one [`omnia::runtime!`] invocation: a static guest
//! from the inline manifest. The static guest is the sole `wasi:cli/run`
//! exporter, so command mode routes to it with no configuration. Because the
//! deployment is compiled in, command mode makes the binary a direct command
//! — no host `run` grammar, the binary's argv belongs to the guest, so it
//! runs as `command-resolver greet Ada`, not
//! `command-resolver run -- greet Ada`; see `README.md`.

cfg_if::cfg_if! {
    if #[cfg(not(target_arch = "wasm32"))] {
        omnia::runtime!({
            mode: command,
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
