//! One locker walked end to end over the raw `omnia:vault` bindings: a
//! host-seeded secret is readable, a stored secret round-trips and is
//! overwritten in place, `exists` distinguishes present from absent,
//! `delete` is idempotent, and `list-ids` reflects the surviving secrets.

#![cfg(target_arch = "wasm32")]

use omnia_wasi_vault::vault;

omnia_guest::command!(scenario);

async fn scenario() {
    let locker = vault::open("locker".to_owned()).await.expect("open");

    // The host seeded `seeded` before the run.
    assert_eq!(locker.get("seeded".to_owned()).await.expect("get"), Some(b"from-host".to_vec()));

    // Set, read back, overwrite, read back.
    locker.set("token".to_owned(), b"v1".to_vec()).await.expect("set");
    assert_eq!(locker.get("token".to_owned()).await.expect("get"), Some(b"v1".to_vec()));
    locker.set("token".to_owned(), b"v2".to_vec()).await.expect("overwrite");
    assert_eq!(locker.get("token".to_owned()).await.expect("get"), Some(b"v2".to_vec()));

    // Exists: present, absent, and a missing get is `None` rather than an error.
    assert!(locker.exists("token".to_owned()).await.expect("exists"));
    assert!(!locker.exists("missing".to_owned()).await.expect("exists"));
    assert_eq!(locker.get("missing".to_owned()).await.expect("get"), None);

    // Delete removes the secret; deleting again is a no-op.
    locker.set("doomed".to_owned(), b"bye".to_vec()).await.expect("set");
    assert!(locker.exists("doomed".to_owned()).await.expect("exists"));
    locker.delete("doomed".to_owned()).await.expect("delete");
    assert!(!locker.exists("doomed".to_owned()).await.expect("exists"));
    assert_eq!(locker.get("doomed".to_owned()).await.expect("get"), None);
    locker.delete("doomed".to_owned()).await.expect("delete absent");

    let mut ids = locker.list_ids().await.expect("list-ids");
    ids.sort();
    assert_eq!(ids, ["seeded", "token"]);
}
