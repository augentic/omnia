use std::fmt::Debug;
use std::ops::Deref;
use std::sync::Arc;

pub use yetti::FutureResult;

use crate::host::generated::wasi::websockets::types::Peer;

/// Providers implement the [`WebSocketServer`] trait to allow the host to
/// interact with backend resources.
pub trait Server: Debug + Send + Sync + 'static {
    /// Get the peers connected to the server.
    fn get_peers(&self) -> Vec<Peer>;

    /// Send a message to the specified peers.
    fn send_peers(&self, message: String, peers: Vec<String>) -> FutureResult<()>;

    /// Send a message to all connected peers.
    fn send_all(&self, message: String) -> FutureResult<()>;

    /// Perform a health check on the server.
    fn health_check(&self) -> FutureResult<String>;
}

#[derive(Clone, Debug)]
pub struct ServerProxy(pub Arc<dyn Server>);

impl Deref for ServerProxy {
    type Target = Arc<dyn Server>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
