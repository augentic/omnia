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

#[cfg(all(target_arch = "wasm32", target_os = "wasi"))]
impl WebSockets for crate::WasiProvider {
    async fn broadcast(&self, data: &[u8]) -> Result<()> {
        send(data, None).await
    }

    async fn send_to(&self, sockets: &[&str], data: &[u8]) -> Result<()> {
        send(data, Some(sockets.iter().map(|&socket| socket.to_owned()).collect())).await
    }
}

#[cfg(all(target_arch = "wasm32", target_os = "wasi"))]
async fn send(data: &[u8], sockets: Option<Vec<String>>) -> Result<()> {
    use omnia_wasi_websocket::{client, types};

    let connection = types::Client::connect("default".to_owned()).await?;
    Ok(client::send(&connection, types::Event::new(data), sockets).await?)
}

#[cfg(all(target_arch = "wasm32", target_os = "wasi"))]
impl From<omnia_wasi_websocket::types::Error> for crate::capability::Error {
    fn from(err: omnia_wasi_websocket::types::Error) -> Self {
        use omnia_wasi_websocket::types::Error as Wire;

        use crate::capability::ErrorKind;

        match err {
            Wire::Timeout => Self::new(ErrorKind::Unavailable, "the socket timed out"),
            Wire::Connection(message) => Self::new(ErrorKind::Unavailable, message),
            Wire::PermissionDenied(message) => Self::new(ErrorKind::Denied, message),
            Wire::Other(message) => Self::new(ErrorKind::Other, message),
        }
    }
}
