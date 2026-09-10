//! One identity walked end to end over the raw `omnia:identity` bindings: a
//! known name resolves and issues a scoped token whose fields cross the
//! boundary intact, and a name the backend refuses surfaces as
//! `internal-failure` rather than trapping.

#![cfg(target_arch = "wasm32")]

use omnia_wasi_identity::credentials::{self, Error};

omnia_guest::command!(scenario);

async fn scenario() {
    let identity = credentials::get_identity("default".to_owned()).await.expect("get-identity");
    let token =
        identity.get_token(vec!["read".to_owned(), "write".to_owned()]).await.expect("get-token");
    assert_eq!(token.token, "recorded-token");
    assert_eq!(token.expires_in, 900);

    // The host maps a backend refusal to `internal-failure` carrying the
    // backend's message; `no-such-identity` is reserved for stale handles.
    match credentials::get_identity("missing".to_owned()).await {
        Err(Error::InternalFailure(message)) => {
            assert!(message.contains("missing"), "refusal lost its message: {message}");
        }
        Err(Error::NoSuchIdentity) => panic!("refusal surfaced as no-such-identity"),
        Ok(_) => panic!("refused identity resolved"),
    }
}
