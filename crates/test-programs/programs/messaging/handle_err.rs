//! A messaging guest whose `incoming-handler` export returns `Err` for every
//! delivery it is handed, naming the topic, so the host side can tell a
//! guest's returned error from a trap.

#![cfg(target_arch = "wasm32")]

use omnia_wasi_messaging::incoming_handler::Guest;
use omnia_wasi_messaging::types::{Delivery, Error};

struct Messaging;
omnia_wasi_messaging::export!(Messaging with_types_in omnia_wasi_messaging);

impl Guest for Messaging {
    async fn handle(delivery: Delivery) -> Result<(), Error> {
        Err(Error::Other(format!("rejected {}", delivery.topic)))
    }
}
