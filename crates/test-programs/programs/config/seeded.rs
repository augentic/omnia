//! Config reads and listings answer from the host's seeded map and nothing
//! else. A seeded key resolves, an empty value is still set, and a variable
//! every test process carries in its environment reads as unset. A listing
//! returns exactly the seeded entries, and pages a prefix one entry at a time.

#![cfg(target_arch = "wasm32")]

use omnia_sdk::config::{self, entries};

omnia_sdk::command::export!(scenario);

async fn scenario() {
    // reads
    let greeting = config::parse::<String>("GREETING").await.expect("seeded key");
    assert_eq!(greeting.as_deref(), Some("hello"));
    let empty = config::parse::<String>("EMPTY").await.expect("seeded key");
    assert_eq!(empty.as_deref(), Some(""), "an empty value is set");
    let path = config::parse::<String>("PATH").await.expect("an unset key is not an error");
    assert_eq!(path, None, "the process environment does not leak");

    // a full listing
    let all = entries::list(String::new(), None, None).await.expect("list every entry");
    assert_eq!(all.next, None, "one page holds every entry");
    assert_eq!(
        sorted(all.entries),
        pairs(&[
            ("EMPTY", ""),
            ("GREETING", "hello"),
            ("ORDERS_CURRENCY", "nzd"),
            ("ORDERS_REGION", "eu"),
        ])
    );

    // a prefix, one entry a page
    let first = entries::list("ORDERS_".to_owned(), None, Some(1)).await.expect("first page");
    assert_eq!(first.entries.len(), 1);
    let after = first.next.expect("a second page");
    let second =
        entries::list("ORDERS_".to_owned(), Some(after), Some(1)).await.expect("second page");
    assert_eq!(second.next, None, "the second page is the last");
    let paged = first.entries.into_iter().chain(second.entries).collect();
    assert_eq!(sorted(paged), pairs(&[("ORDERS_CURRENCY", "nzd"), ("ORDERS_REGION", "eu")]));

    // a prefix that matches no key
    let none = entries::list("MISSING_".to_owned(), None, None).await.expect("an empty listing");
    assert!(none.entries.is_empty(), "no key starts with the prefix");
    assert_eq!(none.next, None);
}

fn sorted(mut entries: Vec<(String, String)>) -> Vec<(String, String)> {
    entries.sort_unstable();
    entries
}

fn pairs(entries: &[(&str, &str)]) -> Vec<(String, String)> {
    entries.iter().map(|&(key, value)| (key.to_owned(), value.to_owned())).collect()
}
