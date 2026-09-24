//! Host-mediated dynamic linking example runtime.
//!
//! Two guests are embedded via the `runtime!` macro's inline manifest keys
//! (the Rust equivalent of `omnia.toml`): `responder` (exports
//! `example:link/echo`) and `router` (imports it, exports `run`). `build.rs`
//! compiles both and names their artifacts in `GUEST_LINK_RESPONDER_WASM`
//! and `GUEST_LINK_ROUTER_WASM`; each entry gives its `name:` because the
//! router dispatches to `responder`, not to the file's stem. The router's
//! import is unsatisfied by its own component and lies outside the runtime's
//! own namespaces, so the host polyfills it on the shared linker and, at
//! bootstrap, serves every guest's linked exports (`omnia::serve_links`, run
//! by `Deployment::assemble`), so a dispatched call always finds the
//! responder's live route. Nothing declares the seam.
//!
//! The router exports a plain `run` rather than an HTTP/messaging trigger;
//! running this binary starts the host and wires the link. See `README.md`.

cfg_if::cfg_if! {
    if #[cfg(not(target_arch = "wasm32"))] {
        use omnia_wasi_http::{WasiHttp, HttpDefault};
        use omnia_wasi_otel::{WasiOtel, OtelDefault};

        omnia::runtime!({
            guests: [
                { name: "responder", path: env!("GUEST_LINK_RESPONDER_WASM") },
                { name: "router", path: env!("GUEST_LINK_ROUTER_WASM") },
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
