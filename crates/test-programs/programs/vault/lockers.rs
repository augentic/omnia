//! Named lockers through the `omnia_sdk` `SecretStore`: the unnamed handle is
//! the `default` locker, a second locker keeps its own secrets under the same
//! key, and `keys` answers in key order and resumes after the key it is given.

#![cfg(target_arch = "wasm32")]

use omnia_sdk::{SecretStore, WasiProvider};

omnia_sdk::api::command::export!(scenario);

async fn scenario() {
    let payments = WasiProvider.named("payments");

    // the same key in each locker holds its own secret
    WasiProvider.set("api-key", b"default-key").await.expect("set in default");
    payments.set("api-key", b"payments-key").await.expect("set in payments");
    let secret = WasiProvider.named("default").get("api-key").await.expect("get from default");
    assert_eq!(secret.as_deref(), Some(&b"default-key"[..]), "the unnamed handle is `default`");
    let secret = payments.get("api-key").await.expect("get from payments");
    assert_eq!(secret.as_deref(), Some(&b"payments-key"[..]));

    // `keys` lists one locker and resumes after a key
    payments.set("c-token", b"c").await.expect("set");
    payments.set("b-token", b"b").await.expect("set");
    let all = payments.keys(None).await.expect("keys");
    assert_eq!(all.items, ["api-key", "b-token", "c-token"]);
    assert_eq!(all.next, None, "a locker lists on one page");
    let rest = payments.keys(Some("api-key")).await.expect("keys after a key");
    assert_eq!(rest.items, ["b-token", "c-token"]);
}
