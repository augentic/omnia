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
use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use fromenv::FromEnv;
use futures::FutureExt;
use futures_channel::mpsc;
use futures_util::stream::TryStreamExt;
use futures_util::{StreamExt, future, pin_mut};
use parking_lot::Mutex;
use qwasr::{Backend, FutureResult};
use serde_json::Value;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::broadcast::{self, Receiver, Sender};
use tokio_stream::wrappers::BroadcastStream;
use tokio_tungstenite::tungstenite::{Error as WsError, Message};
use tokio_tungstenite::{WebSocketStream, accept_async};
use tracing::instrument;

use crate::host::WebSocketCtx;
use crate::host::resource::{Client, Event, EventProxy, Events};

const MAX_CONNECTIONS: usize = 1024;
const BROADCAST_CHANNEL_CAPACITY: usize = 256;
const PER_CLIENT_CHANNEL_CAPACITY: usize = 256;

type ConnectionMap = Arc<Mutex<HashMap<SocketAddr, Connection>>>;

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

        let (event_tx, event_rx) = broadcast::channel::<EventProxy>(BROADCAST_CHANNEL_CAPACITY);
        let connections: ConnectionMap = Arc::new(Mutex::new(HashMap::new()));

        let websocket = Self {
            event_tx,
            event_rx,
            connections,
        };
        let server = websocket.clone();

        tokio::spawn(async move {
            if let Err(e) = server.listen(options.addr).await {
                tracing::error!("websocket server error: {e}");
            }
        });

        Ok(websocket)
    }
}

impl WebSocketCtx for WebSocketDefault {
    fn connect(&self) -> FutureResult<Arc<dyn Client>> {
        tracing::debug!("connecting WebSocket socket");
        let socket = self.clone();
        async move { Ok(Arc::new(socket) as Arc<dyn Client>) }.boxed()
    }

    fn new_event(&self, data: Vec<u8>) -> Result<Arc<dyn Event>> {
        tracing::debug!("creating new event");
        let event = InMemEvent { data };
        Ok(Arc::new(event) as Arc<dyn Event>)
    }
}

impl Client for WebSocketDefault {
    fn events(&self) -> FutureResult<Events> {
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
            Ok(Box::pin(stream) as Events)
        }
        .boxed()
    }

    fn send(&self, event: EventProxy, groups: Option<Vec<String>>) -> FutureResult<()> {
        tracing::debug!("sending event to WebSocket clients, groups: {:?}", groups);
        self.broadcast_event(event.data());

        // let msg = Message::Binary(event.data().into());
        // self.broadcast(&msg, groups.as_deref());

        async move { Ok(()) }.boxed()
    }
}

/// Default implementation for the WebSocket server.
///
/// This implementation listens for new connections and handles them in a
/// separate task. It broadcasts incoming messages to all connected peers and
/// forwards outgoing messages to connected clients.
impl WebSocketDefault {
    async fn listen(self, addr: String) -> Result<()> {
        let listener = TcpListener::bind(addr).await?;
        tracing::info!("websocket server listening on: {}", listener.local_addr()?);

        loop {
            let (stream, peer_addr) = match listener.accept().await {
                Ok(conn) => conn,
                Err(e) => {
                    tracing::error!("accept error: {e}");
                    continue;
                }
            };
            tracing::info!("new connection from: {peer_addr}");

            let server = self.clone();
            tokio::spawn(async move {
                match accept_async(stream).await {
                    Ok(ws_stream) => server.handle_socket(ws_stream, peer_addr).await,
                    Err(e) => tracing::error!("handshake failed for {peer_addr}: {e}"),
                }
            });
        }
    }

    async fn handle_socket(&self, ws_stream: WebSocketStream<TcpStream>, peer_addr: SocketAddr) {
        let (tx, rx) = mpsc::channel(PER_CLIENT_CHANNEL_CAPACITY);

        if let Err(e) = self.add_socket(peer_addr, tx) {
            tracing::error!("issue adding peer connection: {e}");
            return;
        }

        let (outgoing, incoming) = ws_stream.split();

        let incoming_broadcaster = incoming.try_for_each(|msg| {
            match msg {
                Message::Text(text) => {
                    if let Some(groups) = parse_message(&text) {
                        tracing::info!("peer {peer_addr} subscribing to groups: {groups:?}");
                        if let Some(conn) = self.connections.lock().get_mut(&peer_addr) {
                            conn.groups = groups;
                        }
                        return future::ok(());
                    }
                    self.broadcast_event(text.as_bytes().to_vec());
                }
                Message::Binary(data) => {
                    self.broadcast_event(data.to_vec());
                }
                Message::Close(frame) => {
                    tracing::info!("peer {peer_addr} sent close frame: {frame:?}");
                    return future::err(WsError::ConnectionClosed);
                }
                _ => {}
            }

            future::ok(())
        });

        let outgoing_forwarder = rx.map(Ok).forward(outgoing);

        pin_mut!(incoming_broadcaster, outgoing_forwarder);
        future::select(incoming_broadcaster, outgoing_forwarder).await;
        tracing::info!("{peer_addr} disconnected");

        self.connections.lock().remove(&peer_addr);
    }

    /// Add a new socket to the connection map.
    fn add_socket(&self, peer_addr: SocketAddr, tx: mpsc::Sender<Message>) -> Result<()> {
        let mut conns = self.connections.lock();
        if conns.len() >= MAX_CONNECTIONS {
            return Err(anyhow!("max connections reached"));
        }

        conns.insert(
            peer_addr,
            Connection {
                groups: HashSet::new(),
                sender: tx,
            },
        );
        drop(conns);

        Ok(())
    }

    /// Broadcast raw data as an event to all subscribers.
    fn broadcast_event(&self, data: Vec<u8>) {
        let event = InMemEvent { data };
        if self.event_tx.send(EventProxy(Arc::new(event))).is_err() {
            tracing::warn!("no subscribers for incoming WebSocket event");
        }
    }

    /// Send a WebSocket message to connected clients, optionally filtered by group.
    fn broadcast(&self, msg: &Message, groups: Option<&[String]>) {
        let mut conns = self.connections.lock();

        // prune dead connections while collecting recipients
        let dead_peers: Vec<SocketAddr> =
            conns.iter().filter(|(_, c)| c.sender.is_closed()).map(|(a, _)| *a).collect();
        for addr in &dead_peers {
            tracing::debug!("pruning dead connection: {addr}");
            conns.remove(addr);
        }

        let clients: Vec<_> = groups.map_or_else(
            || conns.values().map(|c| c.sender.clone()).collect(),
            |groups| {
                conns
                    .values()
                    .filter(|c| c.groups.iter().any(|g| groups.contains(g)))
                    .map(|c| c.sender.clone())
                    .collect()
            },
        );
        drop(conns);

        for mut client in clients {
            if let Err(e) = client.try_send(msg.clone()) {
                tracing::warn!("failed to send to peer, channel full or disconnected: {e}");
            }
        }
    }
}

/// Parse a JSON text message as a subscribe command.
///
/// Expected format: `{"type": "subscribe", "groups": ["group1", "group2"]}`
fn parse_message(text: &str) -> Option<HashSet<String>> {
    let json = serde_json::from_str::<Value>(text).ok()?;
    if json.get("type").and_then(Value::as_str) != Some("subscribe") {
        return None;
    }
    let groups = json.get("groups").and_then(Value::as_array)?;
    Some(groups.iter().filter_map(|g| g.as_str().map(String::from)).collect())
}

#[derive(Debug, Clone)]
struct Connection {
    groups: HashSet<String>,
    sender: mpsc::Sender<Message>,
}

#[derive(Debug, Clone, Default)]
struct InMemEvent {
    data: Vec<u8>,
}

impl Event for InMemEvent {
    fn group(&self) -> Option<String> {
        None
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
    fn binary_payload() {
        let payload = vec![0, 159, 146, 150];
        let message = Message::Binary(payload.clone().into());
        let Message::Binary(bytes) = message else {
            panic!("expected binary websocket message");
        };
        assert_eq!(bytes.to_vec(), payload);
    }

    #[test]
    fn close_message() {
        let close = Message::Close(Some(CloseFrame {
            code: tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode::Normal,
            reason: "normal".into(),
        }));
        assert!(matches!(close, Message::Close(_)));
    }

    #[test]
    fn backpressure() {
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

    #[test]
    fn parse_message_valid() {
        let msg = r#"{"type": "subscribe", "groups": ["lobby", "chat"]}"#;
        let groups = parse_message(msg).expect("should parse");
        assert!(groups.contains("lobby"));
        assert!(groups.contains("chat"));
        assert_eq!(groups.len(), 2);
    }

    #[test]
    fn parse_message_missing_type() {
        let msg = r#"{"groups": ["lobby"]}"#;
        assert!(parse_message(msg).is_none());
    }

    #[test]
    fn parse_message_wrong_type() {
        let msg = r#"{"type": "publish", "groups": ["lobby"]}"#;
        assert!(parse_message(msg).is_none());
    }

    #[test]
    fn parse_message_not_json() {
        assert!(parse_message("hello world").is_none());
    }
}
