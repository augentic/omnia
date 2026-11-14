//! [`WebSockets`] over `omnia:websocket`.

use omnia_wasi_websocket::{client, types};

use crate::{Error, Result, WasiProvider, WebSockets};

impl WebSockets for WasiProvider {
    async fn broadcast(&self, data: &[u8]) -> Result<()> {
        send(data, None).await
    }

    async fn send_to(&self, sockets: &[&str], data: &[u8]) -> Result<()> {
        send(data, Some(sockets.iter().map(|&socket| socket.to_owned()).collect())).await
    }
}

async fn send(data: &[u8], sockets: Option<Vec<String>>) -> Result<()> {
    let connection = types::Client::connect("default".to_owned()).await?;
    Ok(client::send(&connection, types::Event::new(data), sockets).await?)
}

impl From<types::Error> for Error {
    fn from(err: types::Error) -> Self {
        match err {
            types::Error::Timeout => Self::bad_gateway("unavailable", "the socket timed out"),
            types::Error::Connection(message) => Self::bad_gateway("unavailable", message),
            types::Error::PermissionDenied(message) => Self::server_error("denied", message),
            types::Error::Other(message) => Self::server_error("other", message),
        }
    }
}
