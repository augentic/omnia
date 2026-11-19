//! Named lockers over the `omnia:vault` bindings: a second locker keeps its
//! own secrets under the same key, and `list-secrets` lists one locker in no
//! particular order.

#![cfg(target_arch = "wasm32")]

use omnia_sdk::vault::{DEFAULT, store};

omnia_sdk::api::command::export!(scenario);

async fn scenario() {
    let default = store::open(DEFAULT.to_owned()).await.expect("open default");
    let payments = store::open("payments".to_owned()).await.expect("open payments");

    // the same key in each locker holds its own secret
    default.set("api-key".to_owned(), b"default-key".to_vec()).await.expect("set in default");
    payments.set("api-key".to_owned(), b"payments-key".to_vec()).await.expect("set in payments");
    let secret = default.get("api-key".to_owned()).await.expect("get from default");
    assert_eq!(secret.as_deref(), Some(&b"default-key"[..]));
    let secret = payments.get("api-key".to_owned()).await.expect("get from payments");
    assert_eq!(secret.as_deref(), Some(&b"payments-key"[..]));

    // `list-secrets` lists one locker
    payments.set("c-token".to_owned(), b"c".to_vec()).await.expect("set");
    payments.set("b-token".to_owned(), b"b".to_vec()).await.expect("set");
    let mut all = payments.list_secrets(String::new(), None, None).await.expect("list");
    assert_eq!(all.next, None, "three keys fit on one page");
    all.secrets.sort();
    assert_eq!(all.secrets, ["api-key", "b-token", "c-token"]);
}
