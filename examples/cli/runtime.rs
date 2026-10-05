//! CLI command example runtime.
//!
//! The entire host is one [`omnia::runtime!`] invocation in **command mode**
//! (`mode: command`): it drives the sole `wasi:cli/run` guest exactly once and
//! the generated `main` exits with the guest's status. Command mode rides the
//! same `runtime!` / `TriggerRouter` runtime core every long-lived trigger (HTTP,
//! messaging, …) uses, so re-triggering this same guest from an inbound event
//! tomorrow is a host-wiring change, not a rewrite.
//!
//! It runs through the `omnia` CLI's `run` subcommand, forwarding the guest's
//! argv after `--`; see `README.md`.

cfg_select! {
    not(target_arch = "wasm32") => {
        use omnia_wasi_otel::{OtelDefault, WasiOtel};

        omnia::runtime!({
            mode: command,
            hosts: {
                WasiOtel: OtelDefault,
            },
        });
    }
    _ => {
        fn main() {}
    }
}
