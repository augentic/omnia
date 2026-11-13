//! Listings and ranged reads through the `omnia_sdk` `BlobStore`: `list`
//! filters by prefix and resumes after a name, `get_range` is half-open and
//! clamped to the object, `stat` answers `None` for a missing object, and a
//! call on an object in a container nothing created fails with `NotFound`.

#![cfg(target_arch = "wasm32")]

use std::ops::Range;

use omnia_sdk::bytes::Bytes;
use omnia_sdk::capability::ErrorKind;
use omnia_sdk::{BlobStore, WasiProvider};

omnia_sdk::api::command::export!(scenario);

const LOGS: &str = "logs";
const UNCREATED: &str = "uncreated";
const DIGITS: &[u8] = b"0123456789";

async fn scenario() {
    WasiProvider.create_container(LOGS).await.expect("create");
    for name in ["app/2", "app/1", "db/1", "app/3"] {
        WasiProvider.put(LOGS, name, DIGITS).await.expect("put");
    }

    // `list` filters by prefix and resumes after a name, in name order
    assert_eq!(names("app/", None).await, ["app/1", "app/2", "app/3"]);
    assert_eq!(names("app/", Some("app/1")).await, ["app/2", "app/3"]);
    assert_eq!(names("", Some("app/3")).await, ["db/1"]);

    // `get_range` is half-open and clamped to the object
    assert_eq!(range("app/1", 2..5).await.as_deref(), Some(&b"234"[..]));
    assert_eq!(range("app/1", 0..1).await.as_deref(), Some(&b"0"[..]));
    assert_eq!(range("app/1", 7..100).await.as_deref(), Some(&b"789"[..]));
    assert_eq!(range("app/1", 4..4).await.as_deref(), Some(&b""[..]));
    assert_eq!(range("app/9", 0..1).await, None);

    // `stat` describes an object, and a missing one is `None`
    let stat = WasiProvider.stat(LOGS, "db/1").await.expect("stat").expect("db/1 is stored");
    assert_eq!((stat.name.as_str(), stat.container.as_str(), stat.size), ("db/1", LOGS, 10));
    assert_eq!(WasiProvider.stat(LOGS, "db/9").await.expect("stat"), None);

    // an uncreated container refuses every object call, creates nothing, and deletes
    let refused = [
        WasiProvider.get(UNCREATED, "o").await.map(drop),
        WasiProvider.get_range(UNCREATED, "o", 0..1).await.map(drop),
        WasiProvider.put(UNCREATED, "o", DIGITS).await,
        WasiProvider.delete(UNCREATED, "o").await,
        WasiProvider.stat(UNCREATED, "o").await.map(drop),
        WasiProvider.list(UNCREATED, "", None).await.map(drop),
    ];
    for result in refused {
        let error = result.expect_err("a call on an uncreated container");
        assert_eq!(error.kind(), ErrorKind::NotFound, "{error}");
    }
    assert!(!WasiProvider.container_exists(UNCREATED).await.expect("exists"));
    WasiProvider.delete_container(UNCREATED).await.expect("delete an absent container");
}

async fn names(prefix: &str, after: Option<&str>) -> Vec<String> {
    let page = WasiProvider.list(LOGS, prefix, after).await.expect("list");
    assert_eq!(page.next, None, "a container lists on one page");
    page.items.into_iter().map(|object| object.name).collect()
}

async fn range(name: &str, bounds: Range<u64>) -> Option<Bytes> {
    WasiProvider.get_range(LOGS, name, bounds).await.expect("get range")
}
