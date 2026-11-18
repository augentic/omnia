//! WebSocket send capability.

use std::future::Future;

use crate::Result;

/// Send events to the WebSocket clients connected to a deployment.
#[omnia_guest_macros::capability]
pub trait WebSockets: Send + Sync {
    /// Send `data` to every open connection, in a binary frame.
    fn broadcast(&self, data: &[u8]) -> impl Future<Output = Result<()>> + Send;

    /// Send `data` to each connection named in `sockets`, in a binary frame.
    ///
    /// `sockets` holds the ids a connection's events carry. A connection that
    /// is not open is skipped.
    fn send_to(&self, sockets: &[&str], data: &[u8]) -> impl Future<Output = Result<()>> + Send;
}
