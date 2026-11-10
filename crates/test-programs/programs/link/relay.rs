//! Exports `omnia-test:link/ops` *and* re-imports it: each call consumes one
//! hop from the message (a decimal hop count) and dispatches onward through
//! the host, so a single inbound call produces a chain of arbitrary depth.

#![cfg(target_arch = "wasm32")]

wit_bindgen::generate!({
    world: "relay",
    path: "wit",
});

struct Relay;

export!(Relay);

impl exports::omnia_test::link::ops::Guest for Relay {
    fn ping(target: String, message: String) -> String {
        match message.parse::<u32>() {
            Ok(hops) if hops > 0 => omnia_test::link::ops::ping(&target, &(hops - 1).to_string()),
            _ => format!("{target} relayed to the end"),
        }
    }

    async fn ping_async(target: String, message: String) -> String {
        match message.parse::<u32>() {
            Ok(hops) if hops > 0 => {
                omnia_test::link::ops::ping_async(target, (hops - 1).to_string()).await
            }
            _ => format!("{target} relayed to the end"),
        }
    }
}
