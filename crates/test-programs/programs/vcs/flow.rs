//! Every `omnia_sdk::vcs` fn and method on its success path. Each path lends
//! the `.` mount, a named mount, or a subpath beneath one, and the longest
//! preopen a path names wins.
//!
//! - `open`, `init`, `clone`, and `Workspace::add` lend a path and give a
//!   handle, and `Workspace::remove` takes one away.
//! - The remote methods pass each answer through, a diverged push as its
//!   outcome.
//! - `log_all` and `status_all` walk 1,001 entries, whole and in order.
//! - A path no preopen answers is `denied` before the host is called.

#![cfg(target_arch = "wasm32")]

use omnia_sdk::vcs::{self, ChangeKind};
use test_programs::fails_with;
use wasip3::filesystem::preopens;

omnia_sdk::command::export!(scenario);

// the entries of the long log and status, one more than a page holds
const LONG: usize = 1001;

async fn scenario() {
    assert!(preopens::get_directories().iter().any(|(_, name)| name == "."), "host must mount `.`");

    // the repository at the project mount, and a working copy beneath it
    let project = vcs::open(".").await.expect("open the project");
    let log = project.log_all("emery/rev", Some("sha:base")).await.expect("log");
    let entries: Vec<(&str, &str)> =
        log.iter().map(|entry| (entry.id.as_str(), entry.message.as_str())).collect();
    assert_eq!(
        entries,
        [
            ("l2:emery/rev", "second\n\nSlice: SLICE-002\n"),
            ("l1:emery/rev", "first\n\nSlice: SLICE-001\n"),
        ]
    );
    let dirty = vcs::open("./dirty").await.expect("open a working copy with changes");
    let changes = dirty.status_all().await.expect("status");
    let changes: Vec<(&str, ChangeKind)> =
        changes.iter().map(|change| (change.path.as_str(), change.kind)).collect();
    assert_eq!(
        changes,
        [
            ("a.rs", ChangeKind::Added),
            ("b.rs", ChangeKind::Modified),
            ("c.rs", ChangeKind::Deleted)
        ]
    );

    // repositories laid down here
    vcs::init("./fresh").await.expect("init").expect("a new workspace");
    let added =
        project.add("./.cache/work", "sha:main").await.expect("add").expect("a new workspace");
    added.remove().await.expect("remove");

    // a clone and its remote
    let clone = vcs::clone("https://example.test/repo.git", "./clone", Some(1))
        .await
        .expect("clone")
        .expect("a new workspace");
    clone.fetch("origin").await.expect("fetch");
    assert_eq!(
        clone.remote_bookmark("origin", "emery/rev").await.expect("remote bookmark"),
        Some("rmt:origin:emery/rev".to_owned())
    );
    clone.push("origin", "emery/rev").await.expect("push").expect("a forward push");
    let pushed = clone.push("moved", "emery/rev").await.expect("push");
    let diverged = pushed.expect_err("a push the remote moved past");
    assert_eq!(diverged.current.as_deref(), Some("rmt:moved:emery/rev"));

    // listings longer than a page
    let log = project.log_all("long-0", Some("sha:base")).await.expect("long log");
    let ids: Vec<String> = log.into_iter().map(|entry| entry.id).collect();
    assert_eq!(ids, (0..LONG).map(|n| format!("long-{n}")).collect::<Vec<_>>());
    let big = vcs::open("./big").await.expect("open a working copy with many changes");
    let changes = big.status_all().await.expect("long status");
    let paths: Vec<String> = changes.into_iter().map(|change| change.path).collect();
    assert_eq!(paths, (0..LONG).map(|n| format!("f{n:04}.rs")).collect::<Vec<_>>());

    // a path lends the longest preopen it names, at a `/` boundary
    for path in ["ws/repo", "ws/sub/repo"] {
        let workspace = vcs::open(path).await.expect("open beneath a named mount");
        assert_eq!(workspace.head().await.expect("head"), Some("head-sha".to_owned()), "{path}");
    }

    // a path no preopen answers never reaches the host
    for path in ["/elsewhere", "ws-evil/x", "ws/"] {
        let error = vcs::open(path).await.expect_err("no preopen");
        fails_with(&error, "denied", "matches no preopen");
    }
}
