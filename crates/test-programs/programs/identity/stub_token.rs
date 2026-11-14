//! One identity walked end to end over the raw `omnia:identity` bindings: a
//! known name resolves and issues a scoped token whose fields cross the
//! boundary intact, and a name the backend refuses untyped surfaces as
//! `internal-failure` rather than trapping.

#![cfg(target_arch = "wasm32")]

use omnia_wasi_identity::credentials::{self, Error};

omnia_sdk::api::command::export!(scenario);

async fn scenario() {
    let identity = credentials::get_identity("default".to_owned()).await.expect("get-identity");
    let token =
        identity.get_token(vec!["read".to_owned(), "write".to_owned()]).await.expect("get-token");
    assert_eq!(token.token, "recorded-token");
    assert_eq!(token.expires_in, 900);

    // an untyped refusal keeps the backend's message
    match credentials::get_identity("missing".to_owned()).await {
        Err(Error::InternalFailure(message)) => {
            assert!(message.contains("missing"), "refusal lost its message: {message}");
        }
        Err(Error::NoSuchIdentity) => panic!("refusal surfaced as no-such-identity"),
        Ok(_) => panic!("refused identity resolved"),
    }
}
