use std::collections::HashMap;
use std::convert::Infallible;
use std::env;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex as StdMutex};

use anyhow::Result;
use futures_channel::mpsc::unbounded;
use futures_util::stream::TryStreamExt;
use futures_util::{StreamExt, future, pin_mut};
use hyper::body::Incoming;
use hyper::header::{
    CONNECTION, HeaderValue, SEC_WEBSOCKET_ACCEPT, SEC_WEBSOCKET_KEY, SEC_WEBSOCKET_VERSION,
    UPGRADE,
};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::upgrade::Upgraded;
use hyper::{Method, Request, Response, StatusCode, Version};
use hyper_util::rt::TokioIo;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Mutex, OnceCell};
use tokio_tungstenite::tungstenite::handshake::derive_accept_key;
use tokio_tungstenite::tungstenite::{Bytes, Message, Utf8Bytes};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async};
use tungstenite::protocol::Role;
use yetti::State;

use crate::host::WebSocketsView;
use crate::host::types::{PeerInfo, PeerMap, PublishMessage};

const DEF_WEBSOCKETS_ADDR: &str = "0.0.0.0:80";

static PEER_MAP: OnceCell<PeerMap> = OnceCell::const_new();

static SERVICE_CLIENT: OnceCell<Mutex<WebSocketStream<MaybeTlsStream<TcpStream>>>> =
    OnceCell::const_new();

pub fn get_peer_map() -> Result<PeerMap> {
    let peer_map = PEER_MAP.get().ok_or_else(|| anyhow::anyhow!("Peer map not initialized"))?;
    Ok(Arc::<StdMutex<HashMap<SocketAddr, PeerInfo>>>::clone(peer_map))
}

/// Get the singleton websocket service client
pub async fn service_client() -> &'static Mutex<WebSocketStream<MaybeTlsStream<TcpStream>>> {
    SERVICE_CLIENT
        .get_or_init(|| async {
            let addr = env::var("WEBSOCKETS_ADDR").unwrap_or_else(|_| DEF_WEBSOCKETS_ADDR.into());
            let (client, _) = connect_async(format!("ws://{addr}")).await.unwrap();
            tokio::sync::Mutex::new(client)
        })
        .await
}

/// Send a message to specified peers or all connected peers
/// Accepts a JSON string representing a `PublishMessage`
/// with a peers field (comma-separated list of peer addresses or "all")
/// and a content field (the message to send)
pub fn send_message(message: &str) -> Result<()> {
    let message: PublishMessage = match serde_json::from_str(message) {
        Ok(m) => m,
        Err(e) => {
            tracing::warn!("Failed to parse message: {}", e);
            return Ok(());
        }
    };
    let peer_map = get_peer_map()?;
    let peers = peer_map.lock().unwrap();
    let recipients = if message.peers == "all" {
        peers.values().collect::<Vec<&PeerInfo>>()
    } else {
        let target_peers: Vec<SocketAddr> =
            message.peers.split(',').filter_map(|s| s.parse().ok()).collect();
        let mut filtered_peers: Vec<&PeerInfo> = Vec::new();
        for addr in &target_peers {
            if let Some(peer_info) = peers.get(addr) {
                filtered_peers.push(peer_info);
            }
        }
        filtered_peers
    };

    for recp in recipients {
        if recp.is_service {
            // skip service peers
            continue;
        }
        // we don't really care about the send errors here
        let _ = recp.sender.unbounded_send(Message::Text(Utf8Bytes::from(message.content.clone())));
    }

    Ok(())
}

/// Accept a new websocket connection
#[allow(clippy::significant_drop_tightening)]
async fn accept_connection(
    peer_map: PeerMap, peer: SocketAddr, ws_stream: WebSocketStream<TokioIo<Upgraded>>,
) {
    let (tx, rx) = unbounded();

    let is_service = peer.ip().is_loopback();
    peer_map.lock().unwrap().insert(
        peer,
        PeerInfo {
            is_service,
            query: String::new(),
            sender: tx,
        },
    );

    let (outgoing, incoming) = ws_stream.split();

    let broadcast_incoming = incoming.try_for_each(|msg| {
        if is_service {
            if Message::Ping(Bytes::new()) == msg {
                tracing::info!("Received ping from service peer {}", peer);
            }
            // Ignore all other messages from service peers
            return future::ok(());
        } else if let Message::Text(text) = msg {
            // Handle client filter subscription
            let json_msg: Result<serde_json::Value, _> = serde_json::from_str(&text);
            if json_msg.is_ok() {
                tracing::info!("Setting filter for peer {}: {}", peer, text);
                if let Some(peer_info) = peer_map.lock().unwrap().get_mut(&peer) {
                    peer_info.query = text.to_string();
                }
            } else {
                tracing::error!("Expected filter json object, got unknown text instead: {text}");
                return future::err(tokio_tungstenite::tungstenite::Error::ConnectionClosed);
            }
        }

        future::ok(())
    });

    let receive_from_others = rx.map(Ok).forward(outgoing);

    pin_mut!(broadcast_incoming, receive_from_others);
    future::select(broadcast_incoming, receive_from_others).await;

    tracing::info!("{} disconnected", &peer);
    peer_map.lock().unwrap().remove(&peer);
}

type Body = http_body_util::Full<hyper::body::Bytes>;
/// Handle incoming HTTP requests and upgrade to [`WebSocket`] if appropriate
#[allow(clippy::unused_async)]
#[allow(clippy::map_unwrap_or)]
async fn handle_request(
    peer_map: PeerMap, mut req: Request<Incoming>, addr: SocketAddr,
) -> Result<Response<Body>, Infallible> {
    let upgrade = HeaderValue::from_static("Upgrade");
    let websocket = HeaderValue::from_static("websocket");
    let headers = req.headers();
    let key = headers.get(SEC_WEBSOCKET_KEY);
    let derived = key.map(|k| derive_accept_key(k.as_bytes()));
    if req.method() != Method::GET
        || req.version() < Version::HTTP_11
        || !headers
            .get(CONNECTION)
            .and_then(|h| h.to_str().ok())
            .map(|h| h.split([' ', ',']).any(|p| p.eq_ignore_ascii_case(upgrade.to_str().unwrap())))
            .unwrap_or(false)
        || !headers
            .get(UPGRADE)
            .and_then(|h| h.to_str().ok())
            .map(|h| h.eq_ignore_ascii_case("websocket"))
            .unwrap_or(false)
        || !headers.get(SEC_WEBSOCKET_VERSION).map(|h| h == "13").unwrap_or(false)
        || key.is_none()
        || req.uri() != "/"
    {
        let mut resp =
            Response::new(Body::from("This service only supports WebSocket connections.\n"));
        *resp.status_mut() = StatusCode::BAD_REQUEST;
        return Ok(resp);
    }
    let ver = req.version();
    tokio::task::spawn(async move {
        match hyper::upgrade::on(&mut req).await {
            Ok(upgraded) => {
                let upgraded = TokioIo::new(upgraded);
                accept_connection(
                    peer_map,
                    addr,
                    WebSocketStream::from_raw_socket(upgraded, Role::Server, None).await,
                )
                .await;
            }
            Err(e) => tracing::error!("upgrade error: {e}"),
        }
    });
    let mut res = Response::new(Body::default());
    *res.status_mut() = StatusCode::SWITCHING_PROTOCOLS;
    *res.version_mut() = ver;
    res.headers_mut().append(CONNECTION, upgrade);
    res.headers_mut().append(UPGRADE, websocket);
    res.headers_mut().append(SEC_WEBSOCKET_ACCEPT, derived.unwrap().parse().unwrap());
    Ok(res)
}

#[allow(clippy::missing_errors_doc)]
pub async fn run_server<S>(_: &S) -> Result<()>
where
    S: State,
    S::StoreCtx: WebSocketsView,
{
    let state = PeerMap::new(StdMutex::new(HashMap::new()));
    let _ = PEER_MAP.set(Arc::<StdMutex<HashMap<SocketAddr, PeerInfo>>>::clone(&state));

    let addr = env::var("WEBSOCKETS_ADDR").unwrap_or_else(|_| DEF_WEBSOCKETS_ADDR.into());
    let listener = TcpListener::bind(&addr).await?;
    tracing::info!("websocket server listening on: {}", listener.local_addr()?);

    loop {
        let (stream, peer_addr) = listener.accept().await?;
        let peer = stream.peer_addr().expect("connected streams should have a peer address");
        tracing::info!("Peer address: {}", peer);
        let state_ref = Arc::<StdMutex<HashMap<SocketAddr, PeerInfo>>>::clone(&state);

        tokio::spawn(async move {
            let io = TokioIo::new(stream);

            let service = service_fn(move |req| {
                handle_request(
                    Arc::<StdMutex<HashMap<SocketAddr, PeerInfo>>>::clone(&state_ref),
                    req,
                    peer_addr,
                )
            });

            let conn = http1::Builder::new().serve_connection(io, service).with_upgrades();

            if let Err(err) = conn.await {
                tracing::error!("failed to serve connection: {err:?}");
            }
        });
    }
}
