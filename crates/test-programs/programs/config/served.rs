//! A backend serves the host variables it names and no other. Each argument
//! is a served `NAME=value` pair, or the bare name of a variable the host
//! sets and the backend withholds: a served one reads as its value, a
//! withheld one as unset, and a listing holds the served pairs alone.

#![cfg(target_arch = "wasm32")]

use omnia_sdk::config::{self, entries};

omnia_sdk::command::export!(scenario);

async fn scenario() {
    let arguments = wasip3::cli::environment::get_arguments();
    let (served, withheld): (Vec<&String>, Vec<&String>) =
        arguments.iter().skip(1).partition(|argument| argument.contains('='));
    let served: Vec<(String, String)> = served
        .into_iter()
        .filter_map(|pair| pair.split_once('='))
        .map(|(name, value)| (name.to_owned(), value.to_owned()))
        .collect();
    assert!(!served.is_empty() && !withheld.is_empty(), "the suite names both kinds");

    // reads
    for (name, value) in &served {
        let read = config::get(name).await.expect("a served key");
        assert_eq!(read.as_deref(), Some(value.as_str()), "`{name}` is served");
    }
    for name in withheld {
        let read = config::get(name).await.expect("a withheld key is not an error");
        assert_eq!(read, None, "`{name}` is withheld");
    }

    // a listing
    let all = entries::list(String::new(), None, None).await.expect("list every entry");
    assert_eq!(all.next, None, "one page holds every entry");
    assert_eq!(sorted(all.entries), sorted(served));
}

fn sorted(mut entries: Vec<(String, String)>) -> Vec<(String, String)> {
    entries.sort_unstable();
    entries
}
