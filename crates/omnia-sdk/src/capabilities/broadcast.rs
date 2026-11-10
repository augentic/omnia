//! WebSocket broadcast capability.

use std::future::Future;

use anyhow::Result;

/// Sends events to WebSocket or other broadcast channels.
#[omnia_guest_macros::capability]
pub trait Broadcast: Send + Sync {
    /// Send an event to connected WebSocket clients.
    fn send(
        &self, name: &str, data: &[u8], sockets: Option<Vec<String>>,
    ) -> impl Future<Output = Result<()>> + Send;
}

#[cfg(all(target_arch = "wasm32", target_os = "wasi"))]
impl Broadcast for crate::WasiProvider {
    async fn send(&self, name: &str, data: &[u8], sockets: Option<Vec<String>>) -> Result<()> {
        use anyhow::anyhow;

        let client = omnia_wasi_websocket::types::Client::connect(name.to_string())
            .await
            .map_err(|e| anyhow!("connecting to websocket: {e}"))?;
        let event = omnia_wasi_websocket::types::Event::new(data);
        omnia_wasi_websocket::client::send(&client, event, sockets)
            .await
            .map_err(|e| anyhow!("sending websocket event: {e}"))
    }
}
