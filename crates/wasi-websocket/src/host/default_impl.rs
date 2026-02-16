//! Default implementation for wasi-websocket
//!
//! This implementation runs a real tungstenite WebSocket server that external
//! clients can connect to. Incoming messages from WS clients are broadcast as
//! events to the guest handler. Outbound events from the guest are sent to
//! connected WS clients, optionally filtered by group.
//!
//! For production use, use a backend with proper WebSocket connection
//! management and authentication.

use std::any::Any;
use std::collections::{HashMap, HashSet};
use std::env;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use anyhow::Result;
use futures::FutureExt;
use futures_channel::mpsc;
use futures_util::stream::TryStreamExt;
use futures_util::{StreamExt, future, pin_mut};
use qwasr::{Backend, FutureResult};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::broadcast::{self, Receiver, Sender};
use tokio_stream::wrappers::BroadcastStream;
use tokio_tungstenite::tungstenite::{Message, Utf8Bytes};
use tokio_tungstenite::{WebSocketStream, accept_async};
use tracing::instrument;

use crate::host::WebSocketCtx;
use crate::host::resource::{Event, EventProxy, Socket, Subscriptions};

const DEF_WEBSOCKET_ADDR: &str = "0.0.0.0:80";

/// Options used to connect to the WebSocket service.
#[derive(Debug, Clone, Default)]
pub struct ConnectOptions;

impl qwasr::FromEnv for ConnectOptions {
    fn from_env() -> Result<Self> {
        Ok(Self)
    }
}

/// Default implementation for `wasi:websocket`.
#[derive(Debug)]
pub struct WebSocketDefault {
    event_tx: Sender<EventProxy>,
    event_rx: Receiver<EventProxy>,
    connections: ConnectionMap,
}

impl Clone for WebSocketDefault {
    fn clone(&self) -> Self {
        Self {
            event_tx: self.event_tx.clone(),
            event_rx: self.event_tx.subscribe(),
            connections: Arc::clone(&self.connections),
        }
    }
}

impl Backend for WebSocketDefault {
    type ConnectOptions = ConnectOptions;

    #[instrument]
    #[allow(clippy::used_underscore_binding)]
    async fn connect_with(_options: Self::ConnectOptions) -> Result<Self> {
        tracing::debug!("initializing default WebSocket implementation");
        tracing::warn!("Using default WebSocket implementation - suitable for development only");

        let (event_tx, event_rx) = broadcast::channel::<EventProxy>(256);
        let connections = ConnectionMap::new(Mutex::new(HashMap::new()));

        let websocket = Self {
            event_tx,
            event_rx,
            connections,
        };

        let server = websocket.clone();
        tokio::spawn(async move {
            if let Err(e) = server.listen().await {
                tracing::error!("WebSocket server error: {e}");
            }
        });

        Ok(websocket)
    }
}

impl WebSocketCtx for WebSocketDefault {
    fn connect(&self) -> FutureResult<Arc<dyn Socket>> {
        tracing::debug!("connecting WebSocket socket");
        let socket = self.clone();
        async move { Ok(Arc::new(socket) as Arc<dyn Socket>) }.boxed()
    }

    fn new_event(&self, data: Vec<u8>) -> Result<Arc<dyn Event>> {
        tracing::debug!("creating new event");
        let event = InMemEvent { data, group: None };
        Ok(Arc::new(event) as Arc<dyn Event>)
    }
}

impl Socket for WebSocketDefault {
    fn subscribe(&self) -> FutureResult<Subscriptions> {
        tracing::debug!("subscribing to WebSocket events");
        let stream = BroadcastStream::new(self.event_rx.resubscribe());

        async move {
            let stream = stream.filter_map(|res| async move {
                match res {
                    Ok(event) => Some(event),
                    Err(e) => {
                        tracing::warn!("broadcast lag: {e}");
                        None
                    }
                }
            });
            Ok(Box::pin(stream) as Subscriptions)
        }
        .boxed()
    }

    fn send(&self, event: EventProxy, groups: Option<Vec<String>>) -> FutureResult<()> {
        tracing::debug!("sending event to WebSocket clients, groups: {:?}", groups);
        let connections = Arc::clone(&self.connections);

        async move {
            let data = event.data();
            let text = String::from_utf8(data)
                .unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned());
            let msg = Message::Text(Utf8Bytes::from(text));

            let senders: Vec<_> = {
                let connections =
                    connections.lock().unwrap_or_else(PoisonError::into_inner);
                groups.as_ref().map_or_else(
                    || connections.values().map(|c| c.sender.clone()).collect(),
                    |groups| {
                        connections
                            .values()
                            .filter(|c| groups.iter().any(|g| c.groups.contains(g)))
                            .map(|c| c.sender.clone())
                            .collect()
                    },
                )
            };

            for mut sender in senders {
                if sender.try_send(msg.clone()).is_err() {
                    tracing::warn!("failed to send to peer, channel full or disconnected");
                }
            }

            Ok(())
        }
        .boxed()
    }
}

impl WebSocketDefault {
    async fn listen(self) -> Result<()> {
        let addr = env::var("WEBSOCKET_ADDR").unwrap_or_else(|_| DEF_WEBSOCKET_ADDR.into());
        let listener = TcpListener::bind(&addr).await?;
        tracing::info!("websocket server listening on: {}", listener.local_addr()?);

        loop {
            let (stream, peer_addr) = match listener.accept().await {
                Ok(conn) => conn,
                Err(e) => {
                    tracing::error!("accept error: {e}");
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    continue;
                }
            };
            tracing::info!("New WebSocket connection from: {peer_addr}");

            let server = self.clone();
            tokio::spawn(async move {
                if let Ok(ws_stream) = accept_async(stream).await {
                    server.handle_connect(ws_stream, peer_addr).await;
                } else {
                    tracing::error!("WebSocket handshake failed for {peer_addr}");
                }
            });
        }
    }

    async fn handle_connect(&self, ws_stream: WebSocketStream<TcpStream>, peer: SocketAddr) {
        let (tx, rx) = mpsc::channel(256);

        self.connections
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(
                peer,
                Connection {
                    groups: HashSet::new(),
                    sender: tx,
                },
            );

        let (outgoing, incoming) = ws_stream.split();

        let broadcast_incoming = incoming.try_for_each(|msg| {
            match msg {
                Message::Text(text) => {
                    if let Ok(json) = serde_json::from_str::<serde_json::Value>(&text)
                        && json.get("type").and_then(|t| t.as_str()) == Some("subscribe")
                        && let Some(groups) = json.get("groups").and_then(|g| g.as_array())
                    {
                        let group_set: HashSet<String> =
                            groups.iter().filter_map(|g| g.as_str().map(String::from)).collect();
                        tracing::info!("peer {peer} subscribing to groups: {group_set:?}");
                        if let Some(conn) = self
                            .connections
                            .lock()
                            .unwrap_or_else(PoisonError::into_inner)
                            .get_mut(&peer)
                        {
                            conn.groups = group_set;
                        }
                        return future::ok(());
                    }

                    let event = InMemEvent {
                        data: text.as_bytes().to_vec(),
                        group: None,
                    };
                    if self.event_tx.send(EventProxy(Arc::new(event))).is_err() {
                        tracing::warn!("no subscribers for incoming WebSocket event");
                    }
                }
                Message::Binary(data) => {
                    let event = InMemEvent {
                        data: data.to_vec(),
                        group: None,
                    };
                    if self.event_tx.send(EventProxy(Arc::new(event))).is_err() {
                        tracing::warn!("no subscribers for incoming WebSocket event");
                    }
                }
                Message::Close(frame) => {
                    tracing::info!("peer {peer} sent close frame: {frame:?}");
                }
                _ => {}
            }
            future::ok(())
        });

        let receive_from_host = rx.map(Ok).forward(outgoing);

        pin_mut!(broadcast_incoming, receive_from_host);
        future::select(broadcast_incoming, receive_from_host).await;

        tracing::info!("{peer} disconnected");
        self.connections
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&peer);
    }
}

type ConnectionMap = Arc<Mutex<HashMap<SocketAddr, Connection>>>;

#[derive(Debug, Clone)]
struct Connection {
    groups: HashSet<String>,
    sender: mpsc::Sender<Message>,
}

#[derive(Debug, Clone, Default)]
struct InMemEvent {
    data: Vec<u8>,
    group: Option<String>,
}

impl Event for InMemEvent {
    fn group(&self) -> Option<String> {
        self.group.clone()
    }

    fn data(&self) -> Vec<u8> {
        self.data.clone()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn websocket() {
        let ctx = WebSocketDefault::connect_with(ConnectOptions).await.expect("connect");

        // Test connect
        let _socket = ctx.connect().await.expect("connect socket");

        // Test new_event
        let event = ctx.new_event(b"test payload".to_vec()).expect("new event");
        assert_eq!(event.data(), b"test payload".to_vec());
        assert!(event.group().is_none());
    }
}
