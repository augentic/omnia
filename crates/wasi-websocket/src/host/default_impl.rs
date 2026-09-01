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

use anyhow::{Context as _, Result, anyhow};
use dashmap::DashMap;
use futures::FutureExt;
use futures_channel::mpsc;
use futures_util::stream::TryStreamExt;
use futures_util::{StreamExt, future, pin_mut};
use omnia_core::{Backend, FutureResult};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::broadcast::{self, Sender};
use tokio_stream::wrappers::BroadcastStream;
use tokio_tungstenite::tungstenite::{Error as WsError, Message};
use tokio_tungstenite::{WebSocketStream, accept_async};
use tracing::instrument;

use crate::host::WasiWebSocketCtx;
use crate::host::resource::{Client, Event, Events};

const MAX_CONNECTIONS: usize = 1024;
const BROADCAST_CAPACITY: usize = 256;
const CLIENT_CAPACITY: usize = 256;

type ConnectionMap = Arc<DashMap<String, mpsc::Sender<Message>>>;

/// Options used to connect to the WebSocket service.
#[derive(Debug, Clone)]
pub struct ConnectOptions {
    /// The address to bind the WebSocket server to.
    pub socket_addr: String,
}

impl omnia_core::FromEnv for ConnectOptions {
    fn load_env() -> Result<Self> {
        let socket_addr =
            std::env::var("WEBSOCKET_ADDR").unwrap_or_else(|_| "0.0.0.0:80".to_string());
        Ok(Self { socket_addr })
    }
}

/// Default implementation for `wasi:websocket`.
#[derive(Clone, Debug)]
pub struct WebSocketDefault {
    event_tx: Sender<Event>,
    connections: ConnectionMap,
}

impl Backend for WebSocketDefault {
    type ConnectOptions = ConnectOptions;

    #[instrument]
    async fn connect_with(options: Self::ConnectOptions) -> Result<Self> {
        tracing::debug!("using default WebSocket backend");

        let websocket = Self::new();
        let server = websocket.clone();
        tokio::spawn(async move {
            let listener = match TcpListener::bind(options.socket_addr).await {
                Ok(listener) => listener,
                Err(e) => {
                    tracing::error!("issue starting websocket server: {e}");
                    return;
                }
            };
            server.accept_loop(listener).await;
        });

        Ok(websocket)
    }
}

impl WasiWebSocketCtx for WebSocketDefault {
    fn connect(&self) -> FutureResult<Arc<dyn Client>> {
        let client = self.clone();
        async move { Ok(Arc::new(client) as Arc<dyn Client>) }.boxed()
    }
}

impl Client for WebSocketDefault {
    fn events(&self) -> FutureResult<Events> {
        let stream = BroadcastStream::new(self.event_tx.subscribe());

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
    fn send(&self, event: Event, sockets: Option<Vec<String>>) -> FutureResult<()> {
        tracing::debug!("sending event to WebSocket clients, sockets: {:?}", sockets);

        self.connections.retain(|_, sender| !sender.is_closed());

        let msg = Message::Binary(event.data.into());
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
    fn new() -> Self {
        let (event_tx, _) = broadcast::channel::<Event>(BROADCAST_CAPACITY);
        Self {
            event_tx,
            connections: Arc::new(DashMap::new()),
        }
    }

    /// Build the backend serving on a pre-bound `listener` instead of binding
    /// an address, for callers that must know the port before the server
    /// starts (no drop-and-rebind race).
    ///
    /// # Errors
    ///
    /// Returns an error if the listener cannot be registered with the tokio
    /// runtime.
    pub fn with_listener(listener: std::net::TcpListener) -> Result<Self> {
        listener.set_nonblocking(true).context("setting listener non-blocking")?;
        let listener = TcpListener::from_std(listener).context("registering listener")?;
        let websocket = Self::new();
        let server = websocket.clone();
        tokio::spawn(async move { server.accept_loop(listener).await });
        Ok(websocket)
    }

    async fn accept_loop(self, listener: TcpListener) {
        match listener.local_addr() {
            Ok(addr) => tracing::info!("websocket server listening on: {addr}"),
            Err(e) => tracing::warn!("websocket listener address unavailable: {e}"),
        }

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
        let (tx, rx) = mpsc::channel(CLIENT_CAPACITY);

        if let Err(e) = self.add_socket(socket_addr.clone(), tx) {
            tracing::error!("issue adding peer connection: {e}");
            return;
        }

        let (outgoing, incoming) = ws_stream.split();

        let inbound = incoming.try_for_each(|msg| {
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

        let outbound = rx.map(Ok).forward(outgoing);

        pin_mut!(inbound, outbound);
        future::select(inbound, outbound).await;
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
        let event = Event {
            socket_addr: Some(socket_addr),
            data,
            route: None,
        };
        if let Err(e) = self.event_tx.send(event) {
            tracing::warn!("issue sending WebSocket event: {e}");
        }
    }
}
