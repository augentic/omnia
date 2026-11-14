//! [`Publisher`] over `wasi:messaging`.

use omnia_wasi_messaging::producer;
use omnia_wasi_messaging::types::{self as wire, Client};

use crate::capability::{Error, ErrorKind, Result};
use crate::messaging::{KEY, Message};
use crate::{Publisher, WasiProvider};

impl Publisher for WasiProvider {
    async fn publish(&self, topic: &str, message: &Message) -> Result<()> {
        let client = Client::connect("host".to_owned()).await?;
        let outgoing = wire::Message::new(&message.payload);
        if let Some(content_type) = &message.content_type {
            outgoing.set_content_type(content_type);
        }
        for (name, value) in &message.headers {
            outgoing.add_metadata(name, value);
        }
        if let Some(key) = &message.key {
            outgoing.add_metadata(KEY, key);
        }
        Ok(producer::send(&client, topic.to_owned(), outgoing).await?)
    }
}

impl From<wire::Error> for Error {
    fn from(err: wire::Error) -> Self {
        match err {
            wire::Error::Timeout => Self::new(ErrorKind::Unavailable, "the broker timed out"),
            wire::Error::Connection(message) => Self::new(ErrorKind::Unavailable, message),
            wire::Error::PermissionDenied(message) => Self::new(ErrorKind::Denied, message),
            wire::Error::Other(message) => Self::new(ErrorKind::Other, message),
        }
    }
}
