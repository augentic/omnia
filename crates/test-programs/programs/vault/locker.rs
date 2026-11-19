//! One locker walked end to end over the raw `omnia:vault` bindings, on the
//! one handle `store::open` returns.
//!
//! - An undeclared locker fails to open with `not-found`.
//! - A host-seeded secret is readable, and a set replaces the value there.
//! - An absent secret is `false` and `none`, not an error.
//! - Deleting an absent secret succeeds, and a deleted name may be set again
//!   at once.
//! - `list-secrets` pages the names under a prefix, and a zero `limit` is
//!   `invalid`.

#![cfg(target_arch = "wasm32")]

use omnia_sdk::vault::locker::Locker;
use omnia_sdk::vault::store;
use omnia_sdk::vault::types::Error;

omnia_sdk::api::command::export!(scenario);

async fn scenario() {
    let undeclared = store::open("undeclared".to_owned()).await;
    assert!(matches!(undeclared, Err(Error::NotFound(_))), "{undeclared:?}");
    let locker = store::open("locker".to_owned()).await.expect("open");

    // the host seeded `seeded` before the run, and a set replaces a value
    assert_eq!(value(&locker, "seeded").await.as_deref(), Some(&b"from-host"[..]));
    locker.set("token".to_owned(), b"v1".to_vec()).await.expect("set");
    locker.set("token".to_owned(), b"v2".to_vec()).await.expect("overwrite");
    assert_eq!(value(&locker, "token").await.as_deref(), Some(&b"v2"[..]));

    // an absent secret is not an error
    assert!(locker.exists("token".to_owned()).await.expect("exists"));
    assert!(!locker.exists("missing".to_owned()).await.expect("exists"));
    assert_eq!(value(&locker, "missing").await, None);

    // a delete is idempotent, and frees the name at once
    locker.set("doomed".to_owned(), b"bye".to_vec()).await.expect("set");
    locker.delete("doomed".to_owned()).await.expect("delete");
    locker.delete("doomed".to_owned()).await.expect("delete absent");
    assert!(!locker.exists("doomed".to_owned()).await.expect("exists"));
    locker.set("doomed".to_owned(), b"back".to_vec()).await.expect("set a deleted name");
    assert_eq!(value(&locker, "doomed").await.as_deref(), Some(&b"back"[..]));

    // a listing pages the names under its prefix
    for name in ["tls-cert", "tls-key"] {
        locker.set(name.to_owned(), b"pem".to_vec()).await.expect("set");
    }
    let page = locker.list_secrets("tls-".to_owned(), None, Some(1)).await.expect("list");
    assert_eq!(page.secrets.len(), 1, "{:?}", page.secrets);
    let after = page.next.expect("a second page");
    let rest = locker.list_secrets("tls-".to_owned(), Some(after), Some(1)).await.expect("list");
    assert_eq!(rest.next, None);
    let mut listed = [page.secrets, rest.secrets].concat();
    listed.sort();
    assert_eq!(listed, ["tls-cert", "tls-key"]);

    // refused as `invalid`
    let zero_limit = locker.list_secrets(String::new(), None, Some(0)).await;
    assert!(matches!(zero_limit, Err(Error::Invalid(_))), "{zero_limit:?}");
}

async fn value(locker: &Locker, name: &str) -> Option<Vec<u8>> {
    let value = locker.get(name.to_owned()).await;
    value.unwrap_or_else(|err| panic!("get `{name}`: {err:?}"))
}
