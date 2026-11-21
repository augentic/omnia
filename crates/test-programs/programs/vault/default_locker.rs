//! The `default` locker through the `omnia_sdk::vault` forms that borrow
//! their arguments.
//!
//! - A secret written reads back, and a removed one is gone, even twice.
//! - `names` walks a listing longer than one page, each name once and in
//!   order.
//! - `kept` is left for the host to read back.

#![cfg(target_arch = "wasm32")]

use omnia_sdk::vault;

omnia_sdk::command::export!(scenario);

async fn scenario() {
    let locker = vault::default().await.expect("open the default locker");

    locker.write("token", b"v1").await.expect("write");
    assert_eq!(locker.read("token").await.expect("read").as_deref(), Some(&b"v1"[..]));
    assert!(locker.contains("token").await.expect("contains"));
    locker.remove("token").await.expect("remove");
    locker.remove("token").await.expect("remove an absent secret");
    assert_eq!(locker.read("token").await.expect("read"), None);
    assert!(!locker.contains("token").await.expect("contains"));
    locker.write("kept", b"v").await.expect("write");

    // the host seeded more names under `bulk-` than one page holds
    let names = locker.names("bulk-").await.expect("names");
    assert_eq!(names.len(), 1_001, "names missed a page");
    assert_eq!((names[0].as_str(), names[1_000].as_str()), ("bulk-0000", "bulk-1000"));
    assert!(names.is_sorted_by(|a, b| a < b), "names repeated or reordered a name");
}
