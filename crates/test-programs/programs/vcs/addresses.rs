//! Local addresses the suite's backend locates, as a backend holds a clone's
//! `url` and what a repository records for a remote.
//!
//! - An address beneath the guest's mounts is reached, for a fetch beneath a
//!   read-only mount too.
//! - One outside them is `denied`, whether or not it exists, and so is one
//!   through a link the guest lays out of its mount.
//! - A push to an address beneath a read-only mount is `denied`.
//! - An address beneath a mount that holds no directory is `not-found`.
//!
//! The arguments name a repository outside every mount, then a path beside
//! it that does not exist.

#![cfg(target_arch = "wasm32")]

use omnia_sdk::vcs;
use wasip3::filesystem::preopens;

omnia_sdk::command::export!(scenario);

async fn scenario() {
    let args = wasip3::cli::environment::get_arguments();
    let [_, outside, missing] = args.as_slice() else {
        panic!("expected two outside paths, got {args:?}");
    };

    // a clone from outside the mounts, through a link out of one, or from
    // nothing beneath one
    let directories = preopens::get_directories();
    let (root, _) = directories.iter().find(|(_, name)| name == ".").expect("host mounts `.`");
    root.symlink_at("../repo".to_owned(), "escape".to_owned())
        .await
        .expect("plant a link out of the mount");
    for url in [outside.as_str(), missing.as_str(), "../escape"] {
        let cloned = vcs::clone(url, "./refused", None).await;
        refused(cloned.err(), "denied", "outside the guest's mounts");
    }
    let absent = vcs::clone("../absent", "./refused", None).await;
    refused(absent.err(), "not-found", "holds no directory");

    // a clone, fetches, and pushes beneath the mounts
    let clone = vcs::clone("../origin", "./clone", None)
        .await
        .expect("clone from beneath the mount")
        .expect("a new workspace");
    clone.fetch("inside").await.expect("fetch from beneath the mount");
    refused(clone.fetch("outside").await.err(), "denied", "outside the guest's mounts");
    clone.fetch("readonly").await.expect("fetch from beneath the read-only mount");
    let pushed = clone.push("readonly", "main").await;
    refused(pushed.err(), "denied", "beneath no writable mount");
    clone.push("inside", "main").await.expect("push").expect("a forward push");
}

fn refused(error: Option<omnia_sdk::Error>, code: &str, needle: &str) {
    let error = error.unwrap_or_else(|| panic!("expected a `{code}` refusal, got an answer"));
    assert!(
        error.code() == code && error.description().contains(needle),
        "expected `{code}` with `{needle}`, got {error:?}"
    );
}
