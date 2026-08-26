//! # Linking example — relay guest
//!
//! Exports `omnia:link/echo` *and* re-imports it: each call consumes one hop
//! from the message (a decimal hop count) and dispatches onward through the
//! host, so a single inbound call produces a dispatch chain of arbitrary
//! depth.

#![cfg(target_arch = "wasm32")]

wit_bindgen::generate!({
    world: "relay",
    path: "guest-link/wit",
});

struct Relay;

export!(Relay);

impl exports::omnia::link::echo::Guest for Relay {
    fn echo(target: String, message: String) -> String {
        match message.parse::<u32>() {
            Ok(hops) if hops > 0 => omnia::link::echo::echo(&target, &(hops - 1).to_string()),
            _ => format!("{target} relayed to the end"),
        }
    }

    async fn echo_slow(target: String, message: String) -> String {
        match message.parse::<u32>() {
            Ok(hops) if hops > 0 => {
                omnia::link::echo::echo_slow(target, (hops - 1).to_string()).await
            }
            // Finish on the responder's parked `echo-slow`, so the chain's
            // last hop is genuinely pending (and timeout-sensitive).
            _ => omnia::link::echo::echo_slow("responder".to_owned(), message).await,
        }
    }
}
