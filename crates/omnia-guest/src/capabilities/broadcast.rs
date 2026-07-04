//! WebSocket broadcast capability.

use std::future::Future;

use anyhow::Result;

/// Sends events to WebSocket or other broadcast channels.
pub trait Broadcast: Send + Sync {
    /// Send an event to connected WebSocket clients.
    #[cfg(not(target_arch = "wasm32"))]
    fn send(
        &self, name: &str, data: &[u8], sockets: Option<Vec<String>>,
    ) -> impl Future<Output = Result<()>> + Send;

    /// Send an event to connected WebSocket clients.
    #[cfg(target_arch = "wasm32")]
    fn send(
        &self, name: &str, data: &[u8], sockets: Option<Vec<String>>,
    ) -> impl Future<Output = Result<()>> + Send {
        use anyhow::anyhow;
        async move {
            let client = omnia_wasi_websocket::types::Client::connect(name.to_string())
                .await
                .map_err(|e| anyhow!("connecting to websocket: {e}"))?;
            let event = omnia_wasi_websocket::types::Event::new(data);
            omnia_wasi_websocket::client::send(&client, event, sockets)
                .await
                .map_err(|e| anyhow!("sending websocket event: {e}"))
        }
    }
}
