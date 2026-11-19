//! `omnia:websocket`, the events a guest exchanges with WebSocket clients.
//!
//! A guest sends through a [`Client`](types::Client) connected to a service,
//! [`DEFAULT`] unless the deployment names another. It receives through the
//! handler [`export!`] binds.

pub use crate::bindings::imports::omnia::websocket::{broadcast, connections, types};

/// The service a guest's own [`Client`](types::Client) connects to.
pub const DEFAULT: &str = "default";

#[doc(hidden)]
#[macro_export]
macro_rules! __export_websocket_guest {
    ($ty:ident) => {
        $crate::_export_websocket!($ty);
    };
}

/// Export a type as the guest's `omnia:websocket/incoming-handler`.
///
/// The type implements
/// [`Guest`](crate::exports::websocket::incoming_handler::Guest).
#[doc(inline)]
pub use crate::__export_websocket_guest as export;
