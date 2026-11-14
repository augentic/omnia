//! [`WebSockets`] over `omnia:websocket`.

use omnia_wasi_websocket::{client, types};

use crate::{CapabilityError, ErrorKind, WasiProvider, WebSockets};

impl WebSockets for WasiProvider {
    async fn broadcast(&self, data: &[u8]) -> Result<(), CapabilityError> {
        send(data, None).await
    }

    async fn send_to(&self, sockets: &[&str], data: &[u8]) -> Result<(), CapabilityError> {
        send(data, Some(sockets.iter().map(|&socket| socket.to_owned()).collect())).await
    }
}

async fn send(data: &[u8], sockets: Option<Vec<String>>) -> Result<(), CapabilityError> {
    let connection = types::Client::connect("default".to_owned()).await?;
    Ok(client::send(&connection, types::Event::new(data), sockets).await?)
}

impl From<types::Error> for CapabilityError {
    fn from(err: types::Error) -> Self {
        match err {
            types::Error::Timeout => Self::new(ErrorKind::Unavailable, "the socket timed out"),
            types::Error::Connection(message) => Self::new(ErrorKind::Unavailable, message),
            types::Error::PermissionDenied(message) => Self::new(ErrorKind::Denied, message),
            types::Error::Other(message) => Self::new(ErrorKind::Other, message),
        }
    }
}
