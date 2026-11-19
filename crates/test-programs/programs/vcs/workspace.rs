//! Raw bindings: what a `workspace` handle answers over the scripted
//! backend.
//!
//! - `open` and `init` give handles, and `open` where no workspace is
//!   fails with `not-found`.
//! - `init` where a workspace already is, and `add` or `clone` into a
//!   place that holds anything, are `occupied`.
//! - A lookup that finds nothing is `none`.
//! - Pages of one walk a log and a status, and a zero `limit` is
//!   `invalid`.
//! - A log with no base and a bookmark precondition are `unsupported`.
//! - A merge seals a commit, or names the paths it left in conflict or the
//!   pending ones it would not merge into.
//! - The handle of a removed workspace fails with `not-found`.
//! - A clone of no depth is `invalid`. An unknown remote is `not-found`,
//!   where a bookmark the repository has not seen is `none`. A push the
//!   remote moved past is `diverged`, with the commit last seen there.

#![cfg(target_arch = "wasm32")]

use omnia_sdk::vcs::types::{Error, Location, Precondition};
use omnia_sdk::vcs::workspace::{ChangeKind, LogEntry, Rule, Strategy, Unmerged};
use omnia_sdk::vcs::{remotes, store, workspaces};
use wasip3::filesystem::preopens;
use wasip3::filesystem::types::Descriptor;

omnia_sdk::api::command::export!(scenario);

const BASE: &str = "sha:base";
const URL: &str = "https://example.test/repo.git";

async fn scenario() {
    let directories = preopens::get_directories();
    let root = directories
        .iter()
        .find(|(_, name)| name == ".")
        .map(|(dir, _)| dir)
        .expect("host mounts `.`");

    // handles, a place that already holds a workspace, and one that holds none
    let project = store::open(at(root, "")).await.expect("open the project");
    let fresh = store::init(at(root, "fresh")).await.expect("init").expect("a new workspace");
    assert_eq!(fresh.head().await.expect("head of what init made"), Some("head-sha".to_owned()));
    let existing = store::init(at(root, "existing")).await.expect("init");
    assert_eq!(existing.expect_err("a workspace already there").detail, "existing");
    refused(store::open(at(root, "plain")).await.err(), "not-found", "not a repository");

    // a lookup that finds nothing
    assert_eq!(
        project.resolve("main".to_owned()).await.expect("resolve"),
        Some("sha:main".to_owned())
    );
    assert_eq!(project.resolve("missing".to_owned()).await.expect("resolve"), None);
    assert!(
        project.is_ancestor(BASE.to_owned(), "sha:main".to_owned()).await.expect("is-ancestor")
    );
    assert_eq!(
        project.bookmark("emery/rev".to_owned()).await.expect("bookmark"),
        Some("lbl:emery/rev".to_owned())
    );
    assert_eq!(project.bookmark("missing".to_owned()).await.expect("bookmark"), None);
    let unborn = store::open(at(root, "unborn")).await.expect("open a workspace with no commits");
    assert_eq!(unborn.head().await.expect("head"), None);

    // pages of one
    let first = project
        .log("long-999".to_owned(), Some(BASE.to_owned()), None, Some(1))
        .await
        .expect("first log page");
    assert_eq!(ids(&first.entries), ["long-999"]);
    let last = project
        .log("long-999".to_owned(), Some(BASE.to_owned()), first.next, Some(1))
        .await
        .expect("last log page");
    assert_eq!(ids(&last.entries), ["long-1000"]);
    assert_eq!(last.next, None);

    let dirty = store::open(at(root, "dirty")).await.expect("open a working copy with changes");
    let (mut changes, mut after) = (Vec::new(), None);
    loop {
        let page = dirty.status(after, Some(1)).await.expect("status page");
        assert_eq!(page.changes.len(), 1, "a page of one");
        changes.extend(page.changes);
        after = page.next;
        if after.is_none() {
            break;
        }
    }
    let paths: Vec<&str> = changes.iter().map(|change| change.path.as_str()).collect();
    assert_eq!(paths, ["a.rs", "b.rs", "c.rs"]);
    let kinds: Vec<ChangeKind> = changes.iter().map(|change| change.kind).collect();
    assert!(
        matches!(kinds[..], [ChangeKind::Added, ChangeKind::Modified, ChangeKind::Deleted]),
        "each change keeps its kind"
    );

    refused(
        project.log("long-0".to_owned(), Some(BASE.to_owned()), None, Some(0)).await.err(),
        "invalid",
        "at least 1",
    );
    refused(dirty.status(None, Some(0)).await.err(), "invalid", "at least 1");

    // what this backend cannot serve
    refused(project.log("main".to_owned(), None, None, None).await.err(), "unsupported", "base");
    let when = Some(Precondition::Absent);
    refused(
        project.set_bookmark("main".to_owned(), "sha:main".to_owned(), when).await.err(),
        "unsupported",
        "precondition",
    );
    project
        .set_bookmark("emery/rev".to_owned(), "sha:main".to_owned(), None)
        .await
        .expect("set-bookmark")
        .expect("no precondition to fail");

    // merges, never into pending changes
    let work = store::open(at(root, "work")).await.expect("open a clean working copy");
    assert_eq!(
        work.commit("seal it".to_owned()).await.expect("commit"),
        Some("c:seal it".to_owned())
    );
    let policy = vec![Rule {
        pattern: "*.lock".to_owned(),
        strategy: Strategy::Ours,
    }];
    let merged = work.merge("feature".to_owned(), "merge it".to_owned(), policy).await;
    assert_eq!(merged.expect("merge").ok(), Some("m:feature".to_owned()));
    let conflicted = work.merge("conflicting".to_owned(), "merge it".to_owned(), vec![]).await;
    let unmerged = conflicted.expect("merge").expect_err("a merge left in conflict");
    let Unmerged::Conflicts(paths) = unmerged else {
        panic!("expected the paths in conflict, got {unmerged:?}");
    };
    assert_eq!(paths, ["Cargo.lock", "z.lock"], "in path order");
    let into_pending = dirty.merge("feature".to_owned(), "merge it".to_owned(), vec![]).await;
    let unmerged = into_pending.expect("merge").expect_err("a merge into pending changes");
    let Unmerged::Pending(paths) = unmerged else {
        panic!("expected the pending paths, got {unmerged:?}");
    };
    assert_eq!(paths, ["a.rs", "b.rs", "c.rs"], "in path order");

    // a second workspace, removed, and a place already taken
    let added = workspaces::add(&project, at(root, ".cache/work"), "sha:main".to_owned())
        .await
        .expect("add")
        .expect("a new workspace");
    assert_eq!(added.head().await.expect("head of what add made"), Some("head-sha".to_owned()));
    workspaces::remove(&added).await.expect("remove");
    refused(added.head().await.err(), "not-found", "removed");
    let taken = workspaces::add(&project, at(root, "taken"), "sha:main".to_owned()).await;
    assert_eq!(taken.expect("add").expect_err("a place that holds files").detail, "taken");

    // remotes
    refused(
        remotes::clone(URL.to_owned(), at(root, "clone"), Some(0)).await.err(),
        "invalid",
        "depth",
    );
    let taken = remotes::clone(URL.to_owned(), at(root, "taken"), Some(1)).await;
    assert_eq!(taken.expect("clone").expect_err("a place that holds files").detail, "taken");
    let clone = remotes::clone(URL.to_owned(), at(root, "clone"), Some(1))
        .await
        .expect("clone")
        .expect("a new workspace");
    remotes::fetch(&clone, "origin".to_owned()).await.expect("fetch");
    assert_eq!(
        remotes::bookmark(&clone, "origin".to_owned(), "emery/rev".to_owned())
            .await
            .expect("remote bookmark"),
        Some("rmt:origin:emery/rev".to_owned())
    );
    assert_eq!(
        remotes::bookmark(&clone, "origin".to_owned(), "missing".to_owned())
            .await
            .expect("an unseen bookmark"),
        None
    );
    refused(
        remotes::bookmark(&clone, "unknown".to_owned(), "emery/rev".to_owned()).await.err(),
        "not-found",
        "unknown",
    );
    remotes::push(&clone, "origin".to_owned(), "emery/rev".to_owned())
        .await
        .expect("push")
        .expect("a forward push");
    let pushed = remotes::push(&clone, "moved".to_owned(), "emery/rev".to_owned()).await;
    let diverged = pushed.expect("push").expect_err("a push the remote moved past");
    assert_eq!(diverged.current, Some("rmt:moved:emery/rev".to_owned()));
}

fn at<'a>(root: &'a Descriptor, subpath: &str) -> Location<'a> {
    Location {
        root,
        subpath: subpath.to_owned(),
    }
}

fn ids(entries: &[LogEntry]) -> Vec<&str> {
    entries.iter().map(|entry| entry.id.as_str()).collect()
}

fn refused(error: Option<Error>, case: &str, needle: &str) {
    let error = error.unwrap_or_else(|| panic!("expected a `{case}` refusal, got an answer"));
    let (actual, detail) = match &error {
        Error::NotFound(detail) => ("not-found", detail),
        Error::Denied(detail) => ("denied", detail),
        Error::Invalid(detail) => ("invalid", detail),
        Error::Unsupported(detail) => ("unsupported", detail),
        Error::Unavailable(detail) => ("unavailable", detail),
        Error::Other(detail) => ("other", detail),
    };
    assert!(
        actual == case && detail.contains(needle),
        "expected `{case}` with `{needle}`, got {error:?}"
    );
}
