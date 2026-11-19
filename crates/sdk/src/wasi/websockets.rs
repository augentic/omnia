//! [`WebSockets`] over `omnia:websocket`.

use omnia_wasi_websocket::omnia::types::{self as wire, Client};
use omnia_wasi_websocket::omnia::{broadcast, connections};

use crate::{Result, WasiProvider, WebSockets};

// the service an unnamed handle connects to
const DEFAULT: &str = "default";

impl WebSockets for WasiProvider {
    async fn broadcast(&self, data: &[u8]) -> Result<()> {
        let client = Client::connect(DEFAULT.to_owned()).await?;
        Ok(broadcast::send(&client, wire::Message::Binary(data.to_vec()), Vec::new()).await?)
    }

    async fn send_to(&self, sockets: &[&str], data: &[u8]) -> Result<()> {
        let client = Client::connect(DEFAULT.to_owned()).await?;
        let sockets = sockets.iter().map(|&socket| socket.to_owned()).collect();
        Ok(connections::send(&client, sockets, wire::Message::Binary(data.to_vec())).await?)
    }
}
