//! Host-mediated dynamic linking example runtime.
//!
//! Two guests are registered from `omnia.toml`: `responder` (exports
//! `omnia:link/echo`) and `router` (imports it, exports `run`). The router's
//! import is unsatisfied by its own component — the host polyfills it on the
//! shared linker and, at startup, wires the serve side of every linked interface
//! (`omnia::serve_links`, called inside the generated `start()`), so a dispatched
//! call always finds the responder's in-process wRPC server.
//!
//! The router exports a plain `run` rather than an HTTP/messaging trigger, so the
//! end-to-end dispatch is driven by the integration test
//! (`crates/omnia/tests/guest_link.rs`); running this binary starts the host and
//! wires the link. See `README.md`.

cfg_if::cfg_if! {
    if #[cfg(not(target_arch = "wasm32"))] {
        use omnia_wasi_http::{WasiHttp, HttpDefault};
        use omnia_wasi_otel::{WasiOtel, OtelDefault};

        omnia::runtime!({
            hosts: {
                WasiHttp: HttpDefault,
                WasiOtel: OtelDefault,
            }
        });
    } else {
        fn main() {}
    }
}
