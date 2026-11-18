//! One identity walked end to end over the raw `omnia:identity` bindings: a
//! known name opens and issues a scoped token whose expiry arrives as an
//! instant on the guest's clock, and a name the backend refuses untyped
//! surfaces as `other` rather than trapping.

#![cfg(target_arch = "wasm32")]

use std::time::{SystemTime, UNIX_EPOCH};

use omnia_wasi_identity::omnia::{credentials, types};

omnia_sdk::api::command::export!(scenario);

async fn scenario() {
    let identity = credentials::open("default".to_owned()).await.expect("open");

    let before = now_ms();
    let token =
        identity.get_token(vec!["read".to_owned(), "write".to_owned()]).await.expect("get-token");
    let after = now_ms();
    assert_eq!(token.token, "recorded-token");

    // the backend's 900 seconds, counted from when it answered
    let expected = before + 900_000..=after + 900_000;
    assert!(
        expected.contains(&token.expires_at),
        "expires-at {} not in {expected:?}",
        token.expires_at
    );

    // an untyped refusal keeps the backend's message
    match credentials::open("missing".to_owned()).await {
        Err(types::Error::Other(message)) => {
            assert!(message.contains("missing"), "refusal lost its message: {message}");
        }
        Err(err) => panic!("refusal surfaced as {err:?}"),
        Ok(_) => panic!("refused identity opened"),
    }
}

fn now_ms() -> u64 {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).expect("clock after the epoch");
    u64::try_from(now.as_millis()).expect("milliseconds fit a u64")
}
