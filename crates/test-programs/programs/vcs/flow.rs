//! Every `omnia:vcs` operation through the SDK's `Vcs` trait: each path
//! lends the `.` mount or a subpath beneath it, each answer is the scripted
//! backend's, and a typed refusal and an untyped one lower to the SDK's
//! `Error` as its variant (`not-found`, `diverged`) and as `other`.

#![cfg(target_arch = "wasm32")]

use omnia_sdk::WasiProvider;
use omnia_sdk::vcs::{Change, ChangeKind, CloneOptions, Entry, Error, Rule, Strategy, Vcs as _};
use wasip3::filesystem::preopens;

omnia_sdk::export!(command: scenario);

async fn scenario() {
    assert!(preopens::get_directories().iter().any(|(_, name)| name == "."), "host must mount `.`");

    // store
    assert_eq!(WasiProvider.resolve(".", "main").await.expect("resolve"), "sha:main");
    assert!(WasiProvider.descends(".", "sha:base", "sha:main").await.expect("descends"));
    assert!(!WasiProvider.descends(".", "unrelated", "sha:main").await.expect("descends"));
    assert_eq!(WasiProvider.head("./work").await.expect("head"), "head-sha");
    assert_eq!(
        WasiProvider.commit("./work", "seal it").await.expect("commit"),
        Some("c:seal it".to_owned())
    );
    let policy = [Rule {
        paths: "*.lock".to_owned(),
        strategy: Strategy::Ours,
    }];
    let merged = WasiProvider.merge("./work", "feature", "merge it", &policy).await.expect("merge");
    assert_eq!(merged.commit.as_deref(), Some("m:feature"));
    assert!(merged.conflicts.is_empty());
    let conflicted = WasiProvider
        .merge("./work", "conflicting", "merge it", &[])
        .await
        .expect("conflicted merge");
    assert_eq!(conflicted.commit, None);
    assert_eq!(conflicted.conflicts, ["Cargo.lock"]);
    assert_eq!(
        WasiProvider.log(".", "emery/rev", "sha:base").await.expect("log"),
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

    // workspace
    WasiProvider.init("./fresh").await.expect("init");
    WasiProvider.add(".", "./.cache/work", "sha:main").await.expect("add");
    assert_eq!(
        WasiProvider.pending("./work").await.expect("pending"),
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
    WasiProvider.remove("./.cache/work").await.expect("remove");

    // transport
    WasiProvider
        .clone_repo("https://example.test/repo.git", "./clone", CloneOptions { depth: Some(1) })
        .await
        .expect("clone");
    WasiProvider.fetch("./clone", "origin").await.expect("fetch");
    WasiProvider.label("./clone", "emery/rev", "sha:main").await.expect("label");
    assert_eq!(
        WasiProvider.labelled("./clone", "emery/rev").await.expect("labelled"),
        "lbl:emery/rev"
    );
    assert_eq!(
        WasiProvider.fetched("./clone", "origin", "emery/rev").await.expect("fetched"),
        "rmt:origin:emery/rev"
    );
    WasiProvider.push("./clone", "origin", "emery/rev").await.expect("push");

    // a typed refusal crosses as its variant
    let missing = WasiProvider.resolve(".", "missing").await.expect_err("an unknown revision");
    assert_eq!(missing, Error::NotFound("missing".to_owned()));
    assert_eq!(missing.code(), "not-found");
    let unlabelled =
        WasiProvider.labelled("./clone", "missing").await.expect_err("an unknown label");
    assert_eq!(unlabelled, Error::NotFound("missing".to_owned()));
    let unfetched =
        WasiProvider.fetched("./clone", "origin", "missing").await.expect_err("an unfetched label");
    assert_eq!(unfetched, Error::NotFound("missing".to_owned()));
    let moved =
        WasiProvider.push("./clone", "moved", "emery/rev").await.expect_err("a moved label");
    assert_eq!(moved, Error::Diverged("emery/rev".to_owned()));
    assert_eq!(moved.code(), "diverged");

    // an untyped backend failure lowers to `other`, its detail kept
    let offline =
        WasiProvider.push("./clone", "offline", "emery/rev").await.expect_err("no network");
    assert!(
        matches!(offline, Error::Other(ref detail) if detail.contains("network down")),
        "unexpected: {offline:?}"
    );
    assert_eq!(offline.code(), "other");

    // a path no preopen answers never reaches the host
    let elsewhere = WasiProvider.head("/elsewhere").await.expect_err("no preopen");
    assert!(
        matches!(elsewhere, Error::Other(ref detail) if detail.contains("matches no preopen")),
        "unexpected: {elsewhere:?}"
    );
}
