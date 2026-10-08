//! Every `omnia:vcs` operation through the SDK's `Vcs` trait: each path
//! lends the `.` mount or a subpath beneath it, each answer is the scripted
//! backend's, and a typed refusal and an untyped one lower to the SDK's
//! `Error` as `not-found` and `other`.

#![cfg(target_arch = "wasm32")]

use omnia_sdk::vcs::{Change, ChangeKind, CloneOptions, Error, Rule, Strategy, Vcs as _, WasiVcs};
use wasip3::filesystem::preopens;

omnia_sdk::command!(scenario);

async fn scenario() {
    assert!(preopens::get_directories().iter().any(|(_, name)| name == "."), "host must mount `.`");

    // store
    assert_eq!(WasiVcs.resolve(".", "main").await.expect("resolve"), "sha:main");
    assert_eq!(WasiVcs.head("./work").await.expect("head"), "head-sha");
    assert_eq!(
        WasiVcs.commit("./work", "seal it").await.expect("commit"),
        Some("c:seal it".to_owned())
    );
    let policy = [Rule {
        paths: "*.lock".to_owned(),
        strategy: Strategy::Ours,
    }];
    let merged = WasiVcs.merge("./work", "feature", "merge it", &policy).await.expect("merge");
    assert_eq!(merged.commit.as_deref(), Some("m:feature"));
    assert!(merged.conflicts.is_empty());
    let conflicted =
        WasiVcs.merge("./work", "conflicting", "merge it", &[]).await.expect("conflicted merge");
    assert_eq!(conflicted.commit, None);
    assert_eq!(conflicted.conflicts, ["Cargo.lock"]);

    // workspace
    WasiVcs.init("./fresh").await.expect("init");
    WasiVcs.add(".", "./.cache/work", "sha:main").await.expect("add");
    assert_eq!(
        WasiVcs.pending("./work").await.expect("pending"),
        [
            Change {
                path: "a.rs".to_owned(),
                kind: ChangeKind::Added,
            },
            Change {
                path: "b.rs".to_owned(),
                kind: ChangeKind::Modified,
            },
            Change {
                path: "c.rs".to_owned(),
                kind: ChangeKind::Deleted,
            },
        ]
    );
    WasiVcs.remove("./.cache/work").await.expect("remove");

    // transport
    WasiVcs
        .clone_repo("https://example.test/repo.git", "./clone", CloneOptions { depth: Some(1) })
        .await
        .expect("clone");
    WasiVcs.fetch("./clone", "origin").await.expect("fetch");
    WasiVcs.label("./clone", "emery/rev", "sha:main").await.expect("label");
    WasiVcs.push("./clone", "origin", "emery/rev").await.expect("push");

    // a typed refusal crosses as its variant
    let missing = WasiVcs.resolve(".", "missing").await.expect_err("an unknown revision");
    assert_eq!(missing, Error::NotFound("missing".to_owned()));
    assert_eq!(missing.code(), "not-found");

    // an untyped backend failure lowers to `other`, its detail kept
    let offline = WasiVcs.push("./clone", "offline", "emery/rev").await.expect_err("no network");
    assert!(
        matches!(offline, Error::Other(ref detail) if detail.contains("network down")),
        "unexpected: {offline:?}"
    );
    assert_eq!(offline.code(), "other");

    // a path no preopen answers never reaches the host
    let elsewhere = WasiVcs.head("/elsewhere").await.expect_err("no preopen");
    assert!(
        matches!(elsewhere, Error::Other(ref detail) if detail.contains("matches no preopen")),
        "unexpected: {elsewhere:?}"
    );
}
