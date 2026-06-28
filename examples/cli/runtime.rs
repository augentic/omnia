//! CLI command example runtime.
//!
//! The entire host is one [`omnia::runtime!`] invocation in **command mode**
//! (`command: true`): it drives the sole `wasi:cli/run` guest exactly once and
//! the generated `main` exits with the guest's status. Command mode rides the
//! same `runtime!` / `TriggerRouter` floor every long-lived trigger (HTTP,
//! messaging, …) uses, so re-triggering this same guest from an inbound event
//! tomorrow is a host-wiring change, not a rewrite.
//!
//! It runs through the `omnia` CLI's `run` subcommand, forwarding the guest's
//! argv after `--`; see `README.md`.

cfg_if::cfg_if! {
    if #[cfg(not(target_arch = "wasm32"))] {
        omnia::runtime!({ command: true });
    } else {
        fn main() {}
    }
}
