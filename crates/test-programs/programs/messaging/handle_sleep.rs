//! A messaging guest whose `incoming-handler` export sleeps well past the
//! deployment's guest timeout for every message, so the host side can pin
//! the wall-clock bound on a hung handler.

#![cfg(target_arch = "wasm32")]

use std::time::Duration;

use omnia_wasi_messaging::incoming_handler::Guest;
use omnia_wasi_messaging::types::{Error, Message};

struct Messaging;
omnia_wasi_messaging::export!(Messaging with_types_in omnia_wasi_messaging);

impl Guest for Messaging {
    async fn handle(_message: Message) -> Result<(), Error> {
        // `thread::sleep` is an async host call on wasip2, so the fiber
        // suspends and the host's timeout lands; a busy loop would not be
        // cancellable without the epoch
        std::thread::sleep(Duration::from_secs(2));
        Ok(())
    }
}
