//! A messaging guest whose `incoming-handler` export returns `Err` for every
//! message it is handed, naming the topic, so the host side can tell a
//! guest's returned error from a trap.

#![cfg(target_arch = "wasm32")]

use omnia_wasi_messaging::incoming_handler::Guest;
use omnia_wasi_messaging::types::{Error, Message};

struct Messaging;
omnia_wasi_messaging::export!(Messaging with_types_in omnia_wasi_messaging);

impl Guest for Messaging {
    async fn handle(message: Message) -> Result<(), Error> {
        Err(Error::Other(format!("rejected {}", message.topic().unwrap_or_default())))
    }
}
