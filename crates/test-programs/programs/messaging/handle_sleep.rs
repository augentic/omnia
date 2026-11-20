//! A messaging guest whose `incoming-handler` export sleeps well past the
//! deployment's guest timeout for every delivery, so the host side can pin
//! the wall-clock bound on a hung handler.

#![cfg(target_arch = "wasm32")]

use std::time::Duration;

use omnia_sdk::exports::messaging::incoming_handler::Guest;
use omnia_sdk::messaging::types::{Delivery, Settle};

struct Messaging;
omnia_sdk::exports::messaging::export!(Messaging);

impl Guest for Messaging {
    async fn handle(_delivery: Delivery) -> Result<(), Settle> {
        // `thread::sleep` waits on WASI 0.3's async `wait-for`, which the
        // async-typed `handle` may block on, so the fiber suspends and the
        // host's timeout lands; a busy loop would not be cancellable without
        // the epoch
        std::thread::sleep(Duration::from_secs(2));
        Ok(())
    }
}
