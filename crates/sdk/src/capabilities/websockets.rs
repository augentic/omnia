//! WebSocket send capability.

use std::future::Future;

use crate::capability::Result;

/// Send events to the WebSocket clients connected to a deployment.
#[omnia_guest_macros::capability]
pub trait WebSockets: Send + Sync {
    /// Send `data` to every connected socket.
    fn broadcast(&self, data: &[u8]) -> impl Future<Output = Result<()>> + Send;

    /// Send `data` to the sockets named in `sockets`.
    fn send_to(&self, sockets: &[&str], data: &[u8]) -> impl Future<Output = Result<()>> + Send;
}
