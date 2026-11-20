#![doc = include_str!("../README.md")]

// the `#[instrument]` expansion names `::omnia_sdk`, inside this crate too
#[cfg(target_arch = "wasm32")]
extern crate self as omnia_sdk;

mod bindings;
pub mod blobstore;
pub mod command;
pub mod config;
pub mod docstore;
mod error;
mod handler;
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
pub use crate::handler::{Context, Handler, Handlers, Metadata, Router};

/// The handlers a guest implements by hand, each a `Guest` trait beside the
/// `export!` that binds a type to it.
///
/// A guest that routes deliveries by topic binds its [`Router`] with
/// [`messaging::export!`] instead.
pub mod exports {
    /// `omnia:messaging/incoming-handler`, for a guest that settles every
    /// delivery itself.
    pub mod messaging {
        pub use crate::bindings::messaging::exports::omnia::messaging::incoming_handler;

        #[doc(hidden)]
        #[macro_export]
        macro_rules! __export_messaging_guest {
            ($ty:ident) => {
                $crate::_export_messaging!($ty);
            };
        }

        /// Export a type as the guest's `omnia:messaging/incoming-handler`.
        ///
        /// The type implements [`Guest`](incoming_handler::Guest).
        #[doc(inline)]
        pub use crate::__export_messaging_guest as export;
    }

    /// `omnia:websocket/incoming-handler`, for a guest that answers the
    /// events of its WebSocket clients.
    pub mod websocket {
        pub use crate::bindings::websocket::exports::omnia::websocket::incoming_handler;

        #[doc(hidden)]
        #[macro_export]
        macro_rules! __export_websocket_guest {
            ($ty:ident) => {
                $crate::_export_websocket!($ty);
            };
        }

        /// Export a type as the guest's `omnia:websocket/incoming-handler`.
        ///
        /// The type implements [`Guest`](incoming_handler::Guest).
        #[doc(inline)]
        pub use crate::__export_websocket_guest as export;
    }

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
