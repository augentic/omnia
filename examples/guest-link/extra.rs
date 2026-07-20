//! # Linking example — extra guest
//!
//! Exports `omnia:link/echo` like `responder`, but is *absent from the
//! deployment manifest*: it joins the running deployment through dynamic
//! registration (`Runtime::register`) and is then reachable via the same
//! host-mediated dispatch as any static guest (see `register.rs` and the
//! seam suite's `guest_link` scenarios).
//!
//! Its replies are tagged `from extra` so tests can distinguish it from the
//! responder — e.g. after a deregister + re-register upgrade swap.

#![cfg(target_arch = "wasm32")]

// `generate_all` also generates the `wasi:clocks` import bindings so the
// `echo-slow` await is driven by this component's own async runtime.
wit_bindgen::generate!({
    world: "responder",
    path: "guest-link/wit",
    generate_all,
});

struct Extra;

export!(Extra);

impl exports::omnia::link::echo::Guest for Extra {
    fn echo(target: String, message: String) -> String {
        format!("{target} echoes from extra: {message}")
    }

    async fn echo_slow(target: String, message: String) -> String {
        wasi::clocks::monotonic_clock::wait_for(5_000_000).await; // 5ms
        format!("{target} echoes slowly from extra: {message}")
    }
}
