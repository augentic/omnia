//! WebSocket service contract, value types, and default implementation.

/// A WebSocket event crossing the boundary.
///
/// The host owns event state; backends translate to and from their wire
/// representation at the endpoint seam. Non-exhaustive so a new field is
/// not a breaking change: construct via [`Event::new`] and set fields
/// directly.
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct Event {
    /// The socket address this event was received from, when known.
    pub socket_addr: Option<String>,
    /// The event data.
    pub data: Vec<u8>,
    /// The route key used to select a guest, when the event carries one.
    ///
    /// `None` fans the event into the trigger's catch-all guest (the sole
    /// websocket exporter), preserving single-guest behaviour.
    pub route: Option<String>,
}

impl Event {
    /// Create an event with the given payload.
    #[must_use]
    pub fn new(data: Vec<u8>) -> Self {
        Self {
            data,
            ..Self::default()
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod host {
    use std::fmt::Debug;
    use std::pin::Pin;
    use std::sync::Arc;

    use anyhow::{Context as _, Result, anyhow};
    use dashmap::DashMap;
    use futures::{FutureExt as _, Stream};
    use futures_channel::mpsc;
    use futures_util::stream::TryStreamExt;
    use futures_util::{StreamExt, future, pin_mut};
    use tokio::net::{TcpListener, TcpStream};
    use tokio::sync::broadcast::{self, Sender};
    use tokio_stream::wrappers::BroadcastStream;
    use tokio_tungstenite::tungstenite::{Error as WsError, Message};
    use tokio_tungstenite::{WebSocketStream, accept_async};
    use tracing::instrument;

    use super::Event;
    use crate::{Connect, FromEnv, FutureResult};

    const MAX_CONNECTIONS: usize = 1024;
    const BROADCAST_CAPACITY: usize = 256;
    const CLIENT_CAPACITY: usize = 256;

    type ConnectionMap = Arc<DashMap<String, mpsc::Sender<Message>>>;

    /// Stream of WebSocket events.
    pub type Events = Pin<Box<dyn Stream<Item = Event> + Send>>;

    /// Service contract for `omnia:websocket` backends.
    pub trait WebSocketClient: Debug + Send + Sync + 'static {
        /// Open a WebSocket endpoint handle.
        fn endpoint(&self) -> FutureResult<Arc<dyn Endpoint>>;
    }

    /// Stateful handle to a WebSocket endpoint.
    pub trait Endpoint: Debug + Send + Sync + 'static {
        /// Subscribe to incoming events from WebSocket clients.
        fn events(&self) -> FutureResult<Events>;

        /// Send an event to connected WebSocket clients, optionally filtered by sockets.
        fn send(&self, event: Event, sockets: Option<Vec<String>>) -> FutureResult<()>;
    }

    /// Deprecated alias for [`Endpoint`].
    #[deprecated(note = "use `Endpoint`")]
    pub type Client = dyn Endpoint;

    /// Options used to connect to the WebSocket service.
    #[derive(Debug, Clone)]
    pub struct ConnectOptions {
        /// The address to bind the WebSocket server to.
        pub socket_addr: String,
    }

    impl FromEnv for ConnectOptions {
        fn load_env() -> Result<Self> {
            let socket_addr =
                std::env::var("WEBSOCKET_ADDR").unwrap_or_else(|_| "0.0.0.0:80".to_string());
            Ok(Self { socket_addr })
        }
    }

    /// Default tungstenite-backed implementation of [`WebSocketClient`].
    #[derive(Clone, Debug)]
    pub struct WebSocketDefault {
        event_tx: Sender<Event>,
        connections: ConnectionMap,
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
                        Ok(ws_stream) => {
                            server.handle_socket(ws_stream, sender_addr.to_string()).await;
                        }
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

        fn add_socket(&self, socket_addr: String, tx: mpsc::Sender<Message>) -> Result<()> {
            if self.connections.len() >= MAX_CONNECTIONS {
                return Err(anyhow!("max connections reached"));
            }
            self.connections.insert(socket_addr, tx);
            Ok(())
        }

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
}

#[cfg(not(target_arch = "wasm32"))]
#[expect(deprecated, reason = "re-exporting the deprecated alias")]
pub use self::host::Client;
#[cfg(not(target_arch = "wasm32"))]
pub use self::host::{ConnectOptions, Endpoint, Events, WebSocketClient, WebSocketDefault};
