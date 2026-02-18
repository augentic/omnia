//! Default implementation for wasi-websocket
//!
//! This implementation runs a real tungstenite WebSocket server that external
//! clients can connect to. Incoming messages from WS clients are broadcast as
//! events to the guest handler. Outbound events from the guest are sent to
//! connected WS clients, optionally filtered by group.
//!
//! For production use, use a backend with proper WebSocket connection
//! management and authentication.

use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use dashmap::DashMap;
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
use crate::host::resource::{Client, Event, EventProxy, Events};

const MAX_CONNECTIONS: usize = 1024;
const BROADCAST_CHANNEL_CAPACITY: usize = 256;
const PER_CLIENT_CHANNEL_CAPACITY: usize = 256;

type ConnectionMap = Arc<DashMap<String, mpsc::Sender<Message>>>;

/// Options used to connect to the WebSocket service.
#[derive(Debug, Clone, FromEnv)]
pub struct ConnectOptions {
    /// The address to bind the WebSocket server to.
    #[env(from = "WEBSOCKET_ADDR", default = "0.0.0.0:80")]
    pub socket_addr: String,
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
        tracing::debug!("using default WebSocket backend");

        let (event_tx, event_rx) = broadcast::channel::<EventProxy>(BROADCAST_CHANNEL_CAPACITY);
        let connections: ConnectionMap = Arc::new(DashMap::new());

        let websocket = Self {
            event_tx,
            event_rx,
            connections,
        };
        let server = websocket.clone();

        tokio::spawn(async move {
            if let Err(e) = server.listen(options.socket_addr).await {
                tracing::error!("issue starting websocket server: {e}");
            }
        });

        Ok(websocket)
    }
}

impl WebSocketCtx for WebSocketDefault {
    fn connect(&self) -> FutureResult<Arc<dyn Client>> {
        let client = self.clone();
        async move { Ok(Arc::new(client) as Arc<dyn Client>) }.boxed()
    }

    fn new_event(&self, data: Vec<u8>) -> Result<Arc<dyn Event>> {
        Ok(Arc::new(InMemEvent {
            socket_addr: String::new(),
            data,
        }) as Arc<dyn Event>)
    }
}

impl Client for WebSocketDefault {
    fn events(&self) -> FutureResult<Events> {
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

    /// Send event to WebSocket clients, optionally filtered by group.
    fn send(&self, event: EventProxy, sockets: Option<Vec<String>>) -> FutureResult<()> {
        tracing::debug!("sending event to WebSocket clients, sockets: {:?}", sockets);

        self.connections.retain(|_, sender| !sender.is_closed());

        let msg = Message::Binary(event.data().to_vec().into());
        for mut entry in self.connections.iter_mut() {
            if sockets.as_ref().is_some_and(|s| !s.contains(entry.key())) {
                continue;
            }
            if let Err(e) = entry.value_mut().try_send(msg.clone()) {
                tracing::warn!("failed to send to peer, channel full or disconnected: {e}");
            }
        }

        async move { Ok(()) }.boxed()
    }
}

/// Default implementation for the WebSocket server.
///
/// This implementation listens for new connections and handles them in a
/// separate task. It broadcasts incoming messages to all connected peers and
/// forwards outgoing messages to connected clients.
impl WebSocketDefault {
    async fn listen(self, socket_addr: String) -> Result<()> {
        let listener = TcpListener::bind(socket_addr).await?;
        tracing::info!("websocket server listening on: {}", listener.local_addr()?);

        loop {
            let (stream, sender_addr) = match listener.accept().await {
                Ok(conn) => conn,
                Err(e) => {
                    tracing::error!("accept error: {e}");
                    continue;
                }
            };
            tracing::info!("new connection from: {sender_addr}");

            let server = self.clone();
            tokio::spawn(async move {
                match accept_async(stream).await {
                    Ok(ws_stream) => server.handle_socket(ws_stream, sender_addr.to_string()).await,
                    Err(e) => tracing::error!("handshake failed for {sender_addr}: {e}"),
                }
            });
        }
    }

    async fn handle_socket(&self, ws_stream: WebSocketStream<TcpStream>, socket_addr: String) {
        let (tx, rx) = mpsc::channel(PER_CLIENT_CHANNEL_CAPACITY);

        if let Err(e) = self.add_socket(socket_addr.clone(), tx) {
            tracing::error!("issue adding peer connection: {e}");
            return;
        }

        let (outgoing, incoming) = ws_stream.split();

        let incoming_broadcaster = incoming.try_for_each(|msg| {
            match msg {
                Message::Text(text) => {
                    self.send_to_guest(socket_addr.clone(), text.as_bytes().to_vec());
                }
                Message::Binary(data) => self.send_to_guest(socket_addr.clone(), data.to_vec()),
                Message::Close(_) => {
                    tracing::info!("peer {socket_addr} sent close frame");
                    return future::err(WsError::ConnectionClosed);
                }
                _ => {}
            }
            future::ok(())
        });

        let outgoing_forwarder = rx.map(Ok).forward(outgoing);

        pin_mut!(incoming_broadcaster, outgoing_forwarder);
        future::select(incoming_broadcaster, outgoing_forwarder).await;
        tracing::info!("{socket_addr} disconnected");

        self.connections.remove(&socket_addr);
    }

    /// Add a new socket to the connection map.
    fn add_socket(&self, socket_addr: String, tx: mpsc::Sender<Message>) -> Result<()> {
        if self.connections.len() >= MAX_CONNECTIONS {
            return Err(anyhow!("max connections reached"));
        }
        self.connections.insert(socket_addr, tx);
        Ok(())
    }

    /// Send event to the wasm guest's websocket event handler.
    fn send_to_guest(&self, socket_addr: String, data: Vec<u8>) {
        let event = InMemEvent { socket_addr, data };
        if let Err(e) = self.event_tx.send(EventProxy(Arc::new(event))) {
            tracing::warn!("issue sending WebSocket event: {e}");
        }
    }
}

#[derive(Debug, Clone, Default)]
struct InMemEvent {
    socket_addr: String,
    data: Vec<u8>,
}

impl Event for InMemEvent {
    fn socket_addr(&self) -> Option<&str> {
        if self.socket_addr.is_empty() { None } else { Some(&self.socket_addr) }
    }

    fn data(&self) -> &[u8] {
        &self.data
    }
}

#[cfg(test)]
mod tests {
    use tokio_tungstenite::tungstenite::protocol::CloseFrame;
    use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;

    use super::*;

    #[tokio::test]
    async fn websocket() {
        let ctx = WebSocketDefault::connect_with(ConnectOptions {
            socket_addr: "0.0.0.0:80".into(),
        })
        .await
        .expect("connect");

        // Test connect
        let _client = ctx.connect().await.expect("connect client");

        // Test new_event
        let event = ctx.new_event(b"test payload".to_vec()).expect("new event");
        assert_eq!(event.data(), b"test payload");
        assert!(event.socket_addr().is_none());
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
            code: CloseCode::Normal,
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
}
