//! Host-mediated dynamic linking example runtime.
//!
//! Two guests are compiled in via the `runtime!` macro's inline manifest keys
//! (the Rust equivalent of `omnia.toml`): `responder` (exports
//! `omnia:link/echo`) and `router` (imports it, exports `run`). The router's
//! import is unsatisfied by its own component — the host polyfills it on the
//! shared linker and, at bootstrap, wires the serve side of every linked
//! interface (`omnia::serve_links`, run by `Runtime::new`), so a dispatched
//! call always finds the responder's in-process wRPC server.
//!
//! The router exports a plain `run` rather than an HTTP/messaging trigger, so the
//! end-to-end dispatch is driven by the seam suite
//! (`crates/seam-suite/tests/seam/guest_link.rs`); running this binary starts the
//! host and wires the link. See `README.md`.

cfg_if::cfg_if! {
    if #[cfg(not(target_arch = "wasm32"))] {
        use omnia_wasi_http::{WasiHttp, HttpDefault};
        use omnia_wasi_otel::{WasiOtel, OtelDefault};

        omnia::runtime!({
            guests: [
                {
                    id: "responder",
                    source: concat!(
                        env!("CARGO_MANIFEST_DIR"),
                        "/../target/wasm32-wasip2/debug/examples/guest_link_responder_wasm.wasm",
                    ),
                },
                {
                    id: "router",
                    source: concat!(
                        env!("CARGO_MANIFEST_DIR"),
                        "/../target/wasm32-wasip2/debug/examples/guest_link_router_wasm.wasm",
                    ),
                    link: ["omnia:link/echo"],
                },
            ],
            hosts: {
                WasiHttp: HttpDefault,
                WasiOtel: OtelDefault,
            }
        });
    } else {
        fn main() {}
    }
}
