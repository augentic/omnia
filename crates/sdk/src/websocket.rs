//! `omnia:websocket`, the events a guest exchanges with WebSocket clients.
//!
//! A guest [`connect`]s a [`Client`] to a service, [`DEFAULT`] unless the
//! deployment names another, and sends a [`Message`] through it to the
//! connections it names or to every open one. It receives an [`Event`]
//! through the handler [`exports::websocket::export!`] binds.
//!
//! [`exports::websocket::export!`]: crate::exports::websocket::export!

use crate::Result;
pub use crate::bindings::imports::omnia::websocket::{broadcast, connections, types};
pub use crate::websocket::types::{Client, CloseFrame, ConnectionId, Event, Message};

/// The service a guest's own [`Client`] connects to.
pub const DEFAULT: &str = "default";

/// Connect to the service `name`, a binding the deployment declares.
///
/// # Errors
///
/// The host's failure, under its code: `not-found` for a service the
/// deployment does not declare.
pub async fn connect(name: &str) -> Result<Client> {
    Ok(Client::connect(name.to_owned()).await?)
}

impl Client {
    /// Send `message` to each of `connections`.
    ///
    /// Success means the backend took the message for delivery, not that a
    /// peer received it. A connection that is not open is skipped.
    ///
    /// # Errors
    ///
    /// The host's failure, under its code.
    pub async fn send(&self, connections: &[&str], message: Message) -> Result<()> {
        Ok(connections::send(self, owned(connections), message).await?)
    }

    /// Send `message` to every open connection, except those in `except`.
    ///
    /// Success means the backend took the message for delivery, not that a
    /// peer received it.
    ///
    /// # Errors
    ///
    /// The host's failure, under its code.
    pub async fn broadcast(&self, message: Message, except: &[&str]) -> Result<()> {
        Ok(broadcast::send(self, message, owned(except)).await?)
    }

    /// Close `connection`, sending `frame` to its peer.
    ///
    /// `None` sends a close frame with no status code. Closing a connection
    /// that is not open succeeds.
    ///
    /// # Errors
    ///
    /// The host's failure, under its code: `unsupported` for a `frame` the
    /// backend cannot send.
    pub async fn close(&self, connection: &str, frame: Option<CloseFrame>) -> Result<()> {
        Ok(connections::close(self, connection.to_owned(), frame).await?)
    }

    /// Whether `connection` is open.
    ///
    /// # Errors
    ///
    /// The host's failure, under its code.
    pub async fn exists(&self, connection: &str) -> Result<bool> {
        Ok(connections::exists(self, connection.to_owned()).await?)
    }
}

fn owned(connections: &[&str]) -> Vec<ConnectionId> {
    connections.iter().map(|&connection| connection.to_owned()).collect()
}
