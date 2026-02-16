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
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use fromenv::FromEnv;
use futures::FutureExt;
use futures_channel::mpsc;
use futures_util::stream::TryStreamExt;
use futures_util::{StreamExt, future, pin_mut};
use qwasr::{Backend, FutureResult};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::broadcast::{self, Receiver, Sender};
use tokio_stream::wrappers::BroadcastStream;
use tokio_tungstenite::tungstenite::{Error as WsError, Message};
use tokio_tungstenite::{WebSocketStream, accept_async};
use tracing::instrument;

use crate::host::WebSocketCtx;
use crate::host::resource::{Event, EventProxy, Socket, Subscriptions};

const MAX_CONNECTIONS: usize = 1024;

/// Options used to connect to the WebSocket service.
#[derive(Debug, Clone, FromEnv)]
pub struct ConnectOptions {
    /// The address to bind the WebSocket server to.
    #[env(from = "WEBSOCKET_ADDR", default = "0.0.0.0:80")]
    pub addr: String,
}

impl qwasr::FromEnv for ConnectOptions {
    fn from_env() -> Result<Self> {
        Self::from_env().finalize().context("issue loading connection options")
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
    async fn connect_with(options: Self::ConnectOptions) -> Result<Self> {
        tracing::debug!("initializing default WebSocket implementation");

        let (event_tx, event_rx) = broadcast::channel::<EventProxy>(256);
        let connections = ConnectionMap::new(Mutex::new(HashMap::new()));

        let websocket = Self {
            event_tx,
            event_rx,
            connections,
        };

        let server = websocket.clone();
        tokio::spawn(async move {
            if let Err(e) = server.listen(options.addr).await {
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
            let msg = Message::Binary(data.into());
            let requested_groups: Option<HashSet<&str>> =
                groups.as_ref().map(|group_names| group_names.iter().map(String::as_str).collect());

            let senders: Vec<_> = {
                let connections = connections.lock().unwrap_or_else(PoisonError::into_inner);
                requested_groups.as_ref().map_or_else(
                    || connections.values().map(|c| c.sender.clone()).collect(),
                    |groups| {
                        connections
                            .values()
                            .filter(|c| c.groups.iter().any(|g| groups.contains(g.as_str())))
                            .map(|c| c.sender.clone())
                            .collect()
                    },
                )
            };

            let mut failures = 0usize;
            for mut sender in senders {
                if let Err(e) = sender.try_send(msg.clone()) {
                    failures += 1;
                    tracing::warn!("failed to send to peer, channel full or disconnected: {e}");
                }
            }
            if failures > 0 {
                return Err(anyhow!(
                    "failed to enqueue websocket payload for {failures} connection(s)"
                ));
            }

            Ok(())
        }
        .boxed()
    }
}

impl WebSocketDefault {
    async fn listen(self, addr: String) -> Result<()> {
        let listener = TcpListener::bind(addr).await?;
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

        let accepted = {
            let mut connections = self.connections.lock().unwrap_or_else(PoisonError::into_inner);
            if connections.len() >= MAX_CONNECTIONS {
                tracing::warn!(
                    "rejecting websocket connection from {peer}: max connections ({MAX_CONNECTIONS}) reached"
                );
                false
            } else {
                connections.insert(
                    peer,
                    Connection {
                        groups: HashSet::new(),
                        sender: tx,
                    },
                );
                true
            }
        };
        if !accepted {
            return;
        }

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
                    return future::err(WsError::ConnectionClosed);
                }
                _ => {}
            }
            future::ok(())
        });

        let receive_from_host = rx.map(Ok).forward(outgoing);

        pin_mut!(broadcast_incoming, receive_from_host);
        future::select(broadcast_incoming, receive_from_host).await;

        tracing::info!("{peer} disconnected");
        self.connections.lock().unwrap_or_else(PoisonError::into_inner).remove(&peer);
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
    use tokio_tungstenite::tungstenite::protocol::CloseFrame;

    use super::*;

    #[tokio::test]
    async fn websocket() {
        let ctx = WebSocketDefault::connect_with(ConnectOptions {
            addr: "0.0.0.0:80".into(),
        })
        .await
        .expect("connect");

        // Test connect
        let _socket = ctx.connect().await.expect("connect socket");

        // Test new_event
        let event = ctx.new_event(b"test payload".to_vec()).expect("new event");
        assert_eq!(event.data(), b"test payload".to_vec());
        assert!(event.group().is_none());
    }

    #[test]
    fn outbound_payload_is_binary() {
        let payload = vec![0, 159, 146, 150];
        let message = Message::Binary(payload.clone().into());
        let Message::Binary(bytes) = message else {
            panic!("expected binary websocket message");
        };
        assert_eq!(bytes.to_vec(), payload);
    }

    #[test]
    fn close_message_is_terminal_frame() {
        let close = Message::Close(Some(CloseFrame {
            code: tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode::Normal,
            reason: "normal".into(),
        }));
        assert!(matches!(close, Message::Close(_)));
    }

    #[test]
    fn bounded_channel_surfaces_backpressure() {
        let (mut sender, _receiver) = mpsc::channel::<Message>(1);
        for idx in u8::MIN..=u8::MAX {
            match sender.try_send(Message::Binary(vec![idx].into())) {
                Ok(()) => {}
                Err(err) => {
                    assert!(err.is_full());
                    return;
                }
            }
        }
        panic!("expected backpressure after filling channel");
    }
}
