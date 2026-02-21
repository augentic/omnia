//! # WASI Config WIT implementation

// Bindings for the `wasi:config` world.
// See (<https://github.com/WebAssembly/wasi-config/>)
mod generated {
    #![allow(missing_docs)]

    wit_bindgen::generate!({
        world: "config",
        path: "wit",
        generate_all,
    });
}

pub use self::generated::wasi::config::*;
