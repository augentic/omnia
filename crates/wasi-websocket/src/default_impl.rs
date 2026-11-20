//! Default implementation for `omnia:websocket`.
//!
//! This implementation runs a real tungstenite WebSocket server that external
//! clients can connect to. Each connection gets a random id, and each message
//! a client sends reaches the guest handler as an event in its frame type. The
//! guest sends to one connection or to every open one, closes a connection,
//! and checks whether one is open.
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
use omnia_core::{Connect, FutureResult};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::broadcast::{self, Sender};
use tokio_stream::wrappers::BroadcastStream;
use tokio_tungstenite::tungstenite::protocol::CloseFrame as WsCloseFrame;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::{Error as WsError, Message};
use tokio_tungstenite::{WebSocketStream, accept_async};
use tracing::instrument;

use crate::host::resource::{CloseFrame, Endpoint, Event, Events, Frame};
use crate::host::{Error, WebSocketClient};

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

const MAX_CONNECTIONS: usize = 1024;
const BROADCAST_CAPACITY: usize = 256;
const CLIENT_CAPACITY: usize = 256;

/// Default implementation for `omnia:websocket`.
#[derive(Clone, Debug)]
pub struct WebSocketDefault {
    event_tx: Sender<Event>,
    connections: ConnectionMap,
}

impl WebSocketDefault {
    /// Build the backend without a listener: events come only from in-process
    /// senders and sends reach no peer. A server calls `connect_with`.
    #[must_use]
    pub fn new() -> Self {
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

    async fn handle_socket(&self, ws_stream: WebSocketStream<TcpStream>, peer: String) {
        let connection = match connection_id() {
            Ok(connection) => connection,
            Err(e) => {
                tracing::error!("issue identifying the connection from {peer}: {e:#}");
                return;
            }
        };
        tracing::info!("connection {connection} opened from {peer}");

        let (tx, rx) = mpsc::channel(CLIENT_CAPACITY);
        if let Err(e) = self.add_socket(connection.clone(), tx) {
            tracing::error!("issue adding peer connection: {e}");
            return;
        }

        let (outgoing, incoming) = ws_stream.split();

        let inbound = incoming.try_for_each(|msg| {
            match msg {
                Message::Text(text) => {
                    self.send_to_guest(&connection, Frame::Text, text.as_bytes().to_vec());
                }
                Message::Binary(data) => {
                    self.send_to_guest(&connection, Frame::Binary, data.to_vec());
                }
                Message::Close(_) => {
                    tracing::info!("connection {connection} sent a close frame");
                    return future::err(WsError::ConnectionClosed);
                }
                _ => {}
            }
            future::ok(())
        });

        let outbound = rx.map(Ok).forward(outgoing);

        pin_mut!(inbound, outbound);
        future::select(inbound, outbound).await;
        tracing::info!("connection {connection} closed");

        self.connections.remove(&connection);
    }

    fn add_socket(&self, connection: String, tx: mpsc::Sender<Message>) -> Result<()> {
        if self.connections.len() >= MAX_CONNECTIONS {
            return Err(anyhow!("max connections reached"));
        }
        self.connections.insert(connection, tx);
        Ok(())
    }

    fn send_to_guest(&self, connection: &str, frame: Frame, data: Vec<u8>) {
        let event = Event {
            socket_addr: Some(connection.to_owned()),
            data,
            frame,
            route: None,
        };
        if let Err(e) = self.event_tx.send(event) {
            tracing::warn!("issue sending WebSocket event: {e}");
        }
    }

    // queues `message` for each open connection `to` admits, pruning the closed
    fn fan_out(&self, message: &Message, to: impl Fn(&str) -> bool) {
        self.connections.retain(|_, sender| !sender.is_closed());

        for mut entry in self.connections.iter_mut() {
            if !to(entry.key()) {
                continue;
            }
            if let Err(e) = entry.value_mut().try_send(message.clone()) {
                tracing::warn!("failed to send to peer, channel full or disconnected: {e}");
            }
        }
    }
}

impl Connect for WebSocketDefault {
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

impl Default for WebSocketDefault {
    fn default() -> Self {
        Self::new()
    }
}

impl WebSocketClient for WebSocketDefault {
    fn endpoint(&self) -> FutureResult<Arc<dyn Endpoint>> {
        let client = self.clone();
        async move { Ok(Arc::new(client) as Arc<dyn Endpoint>) }.boxed()
    }
}

impl Endpoint for WebSocketDefault {
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

    fn send(&self, event: Event, sockets: Option<Vec<String>>) -> FutureResult<()> {
        tracing::debug!("sending event to WebSocket clients, sockets: {sockets:?}");

        let sent = ws_message(event).map(|message| {
            self.fan_out(&message, |connection| {
                sockets.as_ref().is_none_or(|sockets| sockets.iter().any(|id| id == connection))
            });
        });
        future::ready(sent).boxed()
    }

    fn broadcast(&self, event: Event, except: Vec<String>) -> FutureResult<()> {
        tracing::debug!("broadcasting event to WebSocket clients, except: {except:?}");

        let sent = ws_message(event).map(|message| {
            self.fan_out(&message, |connection| !except.iter().any(|id| id == connection));
        });
        future::ready(sent).boxed()
    }

    fn close(&self, connection: String, frame: Option<CloseFrame>) -> FutureResult<()> {
        if let Some(frame) = &frame
            && !CloseCode::from(frame.code).is_allowed()
        {
            let error =
                Error::Invalid(format!("close code {} is not one this server sends", frame.code));
            return future::ready(Err(error.into())).boxed();
        }

        let frame = frame.map(|frame| WsCloseFrame {
            code: frame.code.into(),
            reason: frame.reason.into(),
        });
        if let Some((_, mut sender)) = self.connections.remove(&connection)
            && let Err(e) = sender.try_send(Message::Close(frame))
        {
            tracing::warn!("failed to queue a close frame for connection {connection}: {e}");
        }
        future::ready(Ok(())).boxed()
    }

    fn exists(&self, connection: String) -> FutureResult<bool> {
        let open = self.connections.get(&connection).is_some_and(|sender| !sender.is_closed());
        future::ready(Ok(open)).boxed()
    }
}

// a text event travels in a text frame, so its data must be UTF-8
fn ws_message(event: Event) -> Result<Message> {
    match event.frame {
        Frame::Binary => Ok(Message::binary(event.data)),
        Frame::Text => {
            let text = String::from_utf8(event.data)
                .map_err(|e| Error::Invalid(format!("a text event is not UTF-8: {e}")))?;
            Ok(Message::text(text))
        }
    }
}

// 128 random bits, so no two connections share an id, even across restarts
fn connection_id() -> Result<String> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes).context("drawing a connection id")?;
    Ok(format!("{:032x}", u128::from_be_bytes(bytes)))
}

type ConnectionMap = Arc<DashMap<String, mpsc::Sender<Message>>>;
