//! Default implementation for wasi-websockets
//!
//! This is a lightweight implementation providing a basic WebSockets server
//! without persistent connections.
//!
//! For production use, use a backend with proper WebSockets connection
//! management.

#![allow(clippy::used_underscore_binding)]

use std::sync::Arc;

use anyhow::Result;
use futures::FutureExt;
use futures_util::SinkExt;
use tokio_tungstenite::tungstenite::{Bytes, Message};
use tracing::instrument;
use yetti::{Backend, FutureResult};

use crate::host::WebSocketsCtx;
use crate::host::generated::wasi::websockets::types::Peer;
use crate::host::resource::Server;
use crate::host::server::{get_peer_map, send_message, service_client};
use crate::host::types::PublishMessage;

#[derive(Debug, Clone, Default)]
pub struct ConnectOptions;

impl yetti::FromEnv for ConnectOptions {
    fn from_env() -> Result<Self> {
        Ok(Self)
    }
}

#[derive(Debug, Clone)]
pub struct WebSocketsDefault;

impl Backend for WebSocketsDefault {
    type ConnectOptions = ConnectOptions;

    #[instrument]
    async fn connect_with(_options: Self::ConnectOptions) -> Result<Self> {
        tracing::debug!("initializing default WebSocket implementation");
        tracing::warn!("Using default WebSocket implementation - suitable for development only");
        Ok(Self)
    }
}

impl WebSocketsCtx for WebSocketsDefault {
    /// Provide a default WebSockets server.
    ///
    /// This is a basic implementation for development use only.
    fn serve(&self) -> FutureResult<Arc<dyn Server>> {
        async move {
            tracing::debug!("creating default WebSockets server");
            Ok(Arc::new(Self) as Arc<dyn Server>)
        }
        .boxed()
    }
}

impl Server for WebSocketsDefault {
    /// Get the peers connected to the server.
    fn get_peers(&self) -> Vec<Peer> {
        let Ok(peer_map) = get_peer_map() else {
            return vec![];
        };

        peer_map.lock().map_or_else(
            |_| vec![],
            |map| {
                map.iter()
                    .filter(|(_, peer)| !peer.is_service)
                    .map(|(key, peer)| Peer {
                        address: key.to_string(),
                        query: peer.query.clone(),
                    })
                    .collect()
            },
        )
    }

    /// Send a message to the specified peers.
    fn send_peers(&self, message: String, peers: Vec<String>) -> FutureResult<()> {
        tracing::debug!("WebSocket write: {message} for peers: {:?}", peers);

        async move {
            let msg = PublishMessage {
                peers: peers.join(","),
                content: message,
            };
            let msg_str = serde_json::to_string(&msg)?;
            send_message(&msg_str)
        }
        .boxed()
    }

    /// Send a message to all connected peers.
    fn send_all(&self, message: String) -> FutureResult<()> {
        tracing::debug!("WebSocket write: {}", message);
        async move {
            let msg = PublishMessage {
                peers: "all".into(),
                content: message,
            };
            let msg_str = serde_json::to_string(&msg)?;
            send_message(&msg_str)
        }
        .boxed()
    }

    /// Perform a health check on the server.
    fn health_check(&self) -> FutureResult<String> {
        async move {
            let ws_client = service_client().await;
            ws_client.lock().await.send(Message::Ping(Bytes::new())).await?;
            Ok("websockets service is healthy".into())
        }
        .boxed()
    }
}
