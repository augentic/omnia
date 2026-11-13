//! Message publishing capability.

use std::future::Future;

use crate::capability::Result;
use crate::messaging::Message;

/// Publish messages to the topics a deployment's broker serves.
#[omnia_guest_macros::capability]
pub trait Publisher: Send + Sync {
    /// Publish `message` to `topic`.
    fn publish(&self, topic: &str, message: &Message) -> impl Future<Output = Result<()>> + Send;
}

#[cfg(all(target_arch = "wasm32", target_os = "wasi"))]
impl Publisher for crate::WasiProvider {
    async fn publish(&self, topic: &str, message: &Message) -> Result<()> {
        use omnia_wasi_messaging::producer;
        use omnia_wasi_messaging::types::{self as wire, Client};

        let client = Client::connect("host".to_owned()).await?;
        let outgoing = wire::Message::new(&message.payload);
        if let Some(content_type) = &message.content_type {
            outgoing.set_content_type(content_type);
        }
        for (name, value) in &message.headers {
            outgoing.add_metadata(name, value);
        }
        if let Some(key) = &message.key {
            outgoing.add_metadata(crate::messaging::KEY, key);
        }
        Ok(producer::send(&client, topic.to_owned(), outgoing).await?)
    }
}

#[cfg(all(target_arch = "wasm32", target_os = "wasi"))]
impl From<omnia_wasi_messaging::types::Error> for crate::capability::Error {
    fn from(err: omnia_wasi_messaging::types::Error) -> Self {
        use omnia_wasi_messaging::types::Error as Wire;

        use crate::capability::ErrorKind;

        match err {
            Wire::Timeout => Self::new(ErrorKind::Unavailable, "the broker timed out"),
            Wire::Connection(message) => Self::new(ErrorKind::Unavailable, message),
            Wire::PermissionDenied(message) => Self::new(ErrorKind::Denied, message),
            Wire::Other(message) => Self::new(ErrorKind::Other, message),
        }
    }
}
