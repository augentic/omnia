use std::collections::HashMap;
use std::fmt::Debug;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use futures_channel::mpsc::UnboundedSender;
use serde::{Deserialize, Serialize};
use tokio_tungstenite::tungstenite::Message;

#[derive(Debug, Clone)]
pub struct PeerInfo {
    pub is_service: bool,
    pub query: String,
    pub sender: UnboundedSender<Message>,
}

pub type PeerMap = Arc<Mutex<HashMap<SocketAddr, PeerInfo>>>;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PublishMessage {
    pub peers: String,
    pub content: String,
}
