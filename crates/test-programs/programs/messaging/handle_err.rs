//! A messaging guest whose `incoming-handler` export releases every delivery
//! it is handed, naming the topic and asking for a delay, so the host side
//! can tell a guest's settlement from a trap.

#![cfg(target_arch = "wasm32")]

use omnia_wasi_messaging::omnia::incoming_handler::Guest;
use omnia_wasi_messaging::omnia::types::{Delivery, Release, Settle};

struct Messaging;
omnia_wasi_messaging::export!(Messaging);

impl Guest for Messaging {
    async fn handle(delivery: Delivery) -> Result<(), Settle> {
        Err(Settle::Release(Release {
            reason: format!("released {}", delivery.topic),
            delay_ms: Some(250),
        }))
    }
}
