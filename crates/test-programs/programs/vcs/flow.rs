//! Every `omnia_sdk::vcs` fn on its success path. Each path
//! lends the `.` mount, a named mount, or a subpath beneath one, and the
//! longest preopen a path names wins. The SDK keeps its answers over the
//! `workspace` handle:
//!
//! - a lookup that finds nothing is `NotFound`, naming what was asked;
//! - a conflict is a `Merged` with no commit, and a diverged push is
//!   `Diverged`;
//! - a log and a status of 1,001 entries come back whole and in order.

#![cfg(target_arch = "wasm32")]

use omnia_sdk::vcs::{
    self, Change, ChangeKind, CloneOptions, Entry, Error, Merged, Rule, Strategy,
};
use wasip3::filesystem::preopens;

omnia_sdk::command::export!(scenario);

// the entries of the long log and status, one more than a page holds
const LONG: usize = 1001;

async fn scenario() {
    assert!(preopens::get_directories().iter().any(|(_, name)| name == "."), "host must mount `.`");

    // the repository at the project mount
    assert_eq!(vcs::resolve(".", "main").await.expect("resolve"), "sha:main");
    assert!(vcs::descends(".", "sha:base", "sha:main").await.expect("descends"));
    assert!(!vcs::descends(".", "unrelated", "sha:main").await.expect("descends"));
    assert_eq!(
        vcs::log(".", "emery/rev", "sha:base").await.expect("log"),
        [
            Entry {
                id: "l2:emery/rev".to_owned(),
                message: "second\n\nSlice: SLICE-002\n".to_owned(),
            },
            Entry {
                id: "l1:emery/rev".to_owned(),
                message: "first\n\nSlice: SLICE-001\n".to_owned(),
            },
        ]
    );

    // working copies beneath it
    assert_eq!(vcs::head("./work").await.expect("head"), "head-sha");
    assert_eq!(
        vcs::commit("./work", "seal it").await.expect("commit"),
        Some("c:seal it".to_owned())
    );
    let policy = [Rule {
        paths: "*.lock".to_owned(),
        strategy: Strategy::Ours,
    }];
    assert_eq!(
        vcs::merge("./work", "feature", "merge it", &policy).await.expect("merge"),
        Merged {
            commit: Some("m:feature".to_owned()),
            conflicts: vec![],
        }
    );
    assert_eq!(
        vcs::merge("./work", "conflicting", "merge it", &[]).await.expect("conflict"),
        Merged {
            commit: None,
            conflicts: vec!["Cargo.lock".to_owned(), "z.lock".to_owned()],
        }
    );
    assert_eq!(
        vcs::pending("./dirty").await.expect("pending"),
        [
            change("a.rs", ChangeKind::Added),
            change("b.rs", ChangeKind::Modified),
            change("c.rs", ChangeKind::Deleted),
        ]
    );
    assert_eq!(vcs::head("./unborn").await, Err(Error::NotFound("HEAD".to_owned())));

    // repositories laid down here
    vcs::init("./fresh").await.expect("init");
    vcs::add(".", "./.cache/work", "sha:main").await.expect("add");
    vcs::remove("./.cache/work").await.expect("remove");

    // a clone and its remote
    vcs::clone("https://example.test/repo.git", "./clone", CloneOptions { depth: Some(1) })
        .await
        .expect("clone");
    vcs::fetch("./clone", "origin").await.expect("fetch");
    vcs::label("./clone", "emery/rev", "sha:main").await.expect("label");
    assert_eq!(vcs::labelled("./clone", "emery/rev").await.expect("labelled"), "lbl:emery/rev");
    assert_eq!(
        vcs::fetched("./clone", "origin", "emery/rev").await.expect("fetched"),
        "rmt:origin:emery/rev"
    );
    vcs::push("./clone", "origin", "emery/rev").await.expect("push");
    assert_eq!(
        vcs::push("./clone", "moved", "emery/rev").await,
        Err(Error::Diverged("emery/rev".to_owned()))
    );

    // a lookup that finds nothing
    let missing = Err(Error::NotFound("missing".to_owned()));
    assert_eq!(vcs::resolve(".", "missing").await, missing);
    assert_eq!(vcs::labelled("./clone", "missing").await, missing);
    assert_eq!(vcs::fetched("./clone", "origin", "missing").await, missing);

    // listings longer than a page
    let log = vcs::log(".", "long-0", "sha:base").await.expect("long log");
    let ids: Vec<String> = log.into_iter().map(|entry| entry.id).collect();
    assert_eq!(ids, (0..LONG).map(|n| format!("long-{n}")).collect::<Vec<_>>());
    let changes = vcs::pending("./big").await.expect("long status");
    let paths: Vec<String> = changes.into_iter().map(|change| change.path).collect();
    assert_eq!(paths, (0..LONG).map(|n| format!("f{n:04}.rs")).collect::<Vec<_>>());

    // a path lends the longest preopen it names, at a `/` boundary
    assert_eq!(vcs::head("ws/repo").await.expect("head beneath `ws`"), "head-sha");
    assert_eq!(vcs::head("ws/sub/repo").await.expect("head beneath `ws/sub`"), "head-sha");

    // a path no preopen answers never reaches the host
    for path in ["/elsewhere", "ws-evil/x", "ws/"] {
        let error = vcs::head(path).await.expect_err("no preopen");
        assert!(
            matches!(error, Error::Other(ref detail) if detail.contains("matches no preopen")),
            "at `{path}`: {error:?}"
        );
    }
}

fn change(path: &str, kind: ChangeKind) -> Change {
    Change {
        path: path.to_owned(),
        kind,
    }
}
