#![doc = include_str!("../README.md")]

// the `#[instrument]` expansion names `::omnia_sdk`, inside this crate too
#[cfg(target_arch = "wasm32")]
extern crate self as omnia_sdk;

pub mod api;
mod bindings;
pub mod blobstore;
pub mod config;
pub mod docstore;
mod error;
pub mod http;
pub mod identity;
pub mod keyvalue;
mod lend;
#[cfg(feature = "http")]
pub mod mcp;
pub mod messaging;
pub mod model;
pub mod otel;
pub mod plugins;
pub mod sql;
pub mod vault;
pub mod vcs;
pub mod websocket;

pub use omnia_guest_macros::Settings;

pub use crate::bindings::imports::{wit_future, wit_stream};
pub use crate::error::*;

/// The handlers a guest exports, one `Guest` trait for each.
pub mod exports {
    pub use crate::bindings::messaging::exports::omnia::messaging;
    pub use crate::bindings::websocket::exports::omnia::websocket;

    // the path the generated `export!` macros reach their glue through
    #[doc(hidden)]
    pub mod omnia {
        pub use crate::bindings::messaging::exports::omnia::messaging;
        pub use crate::bindings::websocket::exports::omnia::websocket;
    }
}

// the path the transport `export!` macros reach the WASI exports through
#[cfg(all(target_arch = "wasm32", target_os = "wasi"))]
#[doc(hidden)]
pub use wasip3;

#[doc(hidden)]
pub use crate::bindings::messaging::_export_messaging;
#[doc(hidden)]
pub use crate::bindings::websocket::_export_websocket;
