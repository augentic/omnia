//! [`WebSockets`] over `omnia:websocket`.

use omnia_wasi_websocket::types::{self as wire, Client};
use omnia_wasi_websocket::{broadcast, connections};

use crate::{Error, Result, WasiProvider, WebSockets};

// the service an unnamed handle connects to
const DEFAULT: &str = "default";

impl WebSockets for WasiProvider {
    async fn broadcast(&self, data: &[u8]) -> Result<()> {
        let client = Client::connect(DEFAULT.to_owned()).await?;
        Ok(broadcast::send(&client, wire::Message::Binary(data.to_vec()), Vec::new()).await?)
    }

    async fn send_to(&self, sockets: &[&str], data: &[u8]) -> Result<()> {
        let client = Client::connect(DEFAULT.to_owned()).await?;
        for &socket in sockets {
            let message = wire::Message::Binary(data.to_vec());
            connections::send(&client, socket.to_owned(), message).await?;
        }
        Ok(())
    }
}

impl From<wire::Error> for Error {
    fn from(err: wire::Error) -> Self {
        match err {
            wire::Error::NotFound(message) => Self::server_error("not-found", message),
            wire::Error::Denied(message) => Self::server_error("denied", message),
            wire::Error::Invalid(message) => Self::server_error("invalid", message),
            wire::Error::Unsupported(message) => Self::server_error("unsupported", message),
            wire::Error::Unavailable(message) => Self::bad_gateway("unavailable", message),
            wire::Error::Other(message) => Self::server_error("other", message),
        }
    }
}
