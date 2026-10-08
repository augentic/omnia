//! End-to-end tests for `omnia:vcs`: every scenario runs a guest component
//! from `crates/test-programs` through the omnia runtime over a recording
//! backend. The guest asserts what crosses the boundary and traps on
//! failure; the host asserts the path every location resolved to and the
//! calls the backend saw, in order.

#![cfg(not(target_arch = "wasm32"))]

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::anyhow;
use futures::FutureExt as _;
use omnia::{ExitStatus, Mount};
use omnia_test::host::{Backends, Deployment, scratch};
use omnia_wasi_vcs::{
    Change, ChangeKind, CloneOptions, Error, FutureResult, Merged, Rule, WasiVcs, WasiVcsCtx,
};
use parking_lot::Mutex;

// Every guest program in `crates/test-programs` must have a matching test
// here; a new program without one fails to compile.
test_programs::foreach_vcs!();

// ------------------------------------------------------------------------
// Harness
// ------------------------------------------------------------------------

async fn run_guest(wasm: &str, mounts: Vec<Mount>) -> Recorder {
    let recorder = Recorder::default();
    let backends = Backends::defaults().await.vcs(recorder.clone());
    let status = Deployment::new()
        .guest("guest", wasm)
        .mounts(mounts)
        .run_host::<WasiVcs, _>(backends)
        .await
        .expect("guest runs");
    assert_eq!(status, ExitStatus::SUCCESS, "guest `{wasm}` failed");
    recorder
}

// ------------------------------------------------------------------------
// Scenario backend
// ------------------------------------------------------------------------

// Records every call with the host paths the locations resolved to and
// answers each from a fixed script: `resolve("missing")` is a typed
// `not-found`, `push(.., "offline", ..)` an untyped failure, and a merge of
// `conflicting` comes back in conflict.
#[derive(Clone, Debug, Default)]
struct Recorder {
    calls: Arc<Mutex<Vec<String>>>,
}

impl Recorder {
    fn record(&self, call: String) {
        self.calls.lock().push(call);
    }

    fn calls(&self) -> Vec<String> {
        self.calls.lock().clone()
    }
}

fn shown(path: &Path) -> String {
    path.display().to_string()
}

impl WasiVcsCtx for Recorder {
    fn resolve(&self, repo: PathBuf, revision: String) -> FutureResult<String> {
        self.record(format!("resolve {} {revision}", shown(&repo)));
        async move {
            if revision == "missing" {
                return Err(Error::NotFound(revision).into());
            }
            Ok(format!("sha:{revision}"))
        }
        .boxed()
    }

    fn head(&self, at: PathBuf) -> FutureResult<String> {
        self.record(format!("head {}", shown(&at)));
        async { Ok("head-sha".to_owned()) }.boxed()
    }

    fn commit(&self, at: PathBuf, message: String) -> FutureResult<Option<String>> {
        self.record(format!("commit {} {message:?}", shown(&at)));
        async move { Ok(Some(format!("c:{message}"))) }.boxed()
    }

    fn merge(
        &self, at: PathBuf, revision: String, message: String, policy: Vec<Rule>,
    ) -> FutureResult<Merged> {
        let rules: Vec<String> =
            policy.iter().map(|rule| format!("{}={:?}", rule.paths, rule.strategy)).collect();
        self.record(format!("merge {} {revision} {message:?} [{}]", shown(&at), rules.join(",")));
        async move {
            if revision == "conflicting" {
                return Ok(Merged {
                    commit: None,
                    conflicts: vec!["Cargo.lock".to_owned()],
                });
            }
            Ok(Merged {
                commit: Some(format!("m:{revision}")),
                conflicts: vec![],
            })
        }
        .boxed()
    }

    fn init(&self, at: PathBuf) -> FutureResult<()> {
        self.record(format!("init {}", shown(&at)));
        async { Ok(()) }.boxed()
    }

    fn add(&self, repo: PathBuf, at: PathBuf, revision: String) -> FutureResult<()> {
        self.record(format!("add {} {} {revision}", shown(&repo), shown(&at)));
        async { Ok(()) }.boxed()
    }

    fn remove(&self, at: PathBuf) -> FutureResult<()> {
        self.record(format!("remove {}", shown(&at)));
        async { Ok(()) }.boxed()
    }

    fn pending(&self, at: PathBuf) -> FutureResult<Vec<Change>> {
        self.record(format!("pending {}", shown(&at)));
        async {
            Ok(vec![
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
            ])
        }
        .boxed()
    }

    fn clone_repo(&self, url: String, at: PathBuf, options: CloneOptions) -> FutureResult<()> {
        self.record(format!("clone {url} {} depth={:?}", shown(&at), options.depth));
        async { Ok(()) }.boxed()
    }

    fn fetch(&self, repo: PathBuf, remote: String) -> FutureResult<()> {
        self.record(format!("fetch {} {remote}", shown(&repo)));
        async { Ok(()) }.boxed()
    }

    fn label(&self, repo: PathBuf, name: String, revision: String) -> FutureResult<()> {
        self.record(format!("label {} {name} {revision}", shown(&repo)));
        async { Ok(()) }.boxed()
    }

    fn push(&self, repo: PathBuf, remote: String, label: String) -> FutureResult<()> {
        self.record(format!("push {} {remote} {label}", shown(&repo)));
        async move {
            if remote == "offline" {
                return Err(anyhow!("network down"));
            }
            Ok(())
        }
        .boxed()
    }
}

// ------------------------------------------------------------------------
// Scenarios
// ------------------------------------------------------------------------

#[tokio::test]
async fn vcs_flow() {
    let project = scratch();
    fs::create_dir(project.path().join("work")).expect("creating the working copy");
    let root = shown(project.path());
    let under = |subpath: &str| shown(&project.path().join(subpath));

    let recorder = run_guest(test_programs::VCS_FLOW, vec![project.mount(true)]).await;

    // Every location resolved beneath the mount, an existing subpath and an
    // absent one alike, and the refused calls reached the backend as put.
    assert_eq!(
        recorder.calls(),
        [
            format!("resolve {root} main"),
            format!("head {}", under("work")),
            format!("commit {} \"seal it\"", under("work")),
            format!("merge {} feature \"merge it\" [*.lock=Strategy::Ours]", under("work")),
            format!("merge {} conflicting \"merge it\" []", under("work")),
            format!("init {}", under("fresh")),
            format!("add {root} {} sha:main", under(".cache/work")),
            format!("pending {}", under("work")),
            format!("remove {}", under(".cache/work")),
            format!("clone https://example.test/repo.git {} depth=Some(1)", under("clone")),
            format!("fetch {} origin", under("clone")),
            format!("label {} emery/rev sha:main", under("clone")),
            format!("push {} origin emery/rev", under("clone")),
            format!("resolve {root} missing"),
            format!("push {} offline emery/rev", under("clone")),
        ]
    );
}

#[tokio::test]
async fn vcs_locations() {
    let project = scratch();
    fs::create_dir(project.path().join("nested")).expect("creating nested dir");
    let readonly = scratch();

    let recorder = run_guest(
        test_programs::VCS_LOCATIONS,
        vec![project.mount(true), readonly.mount_as("ro", false)],
    )
    .await;

    // Only the read on the read-only mount and the init at the new subpath
    // reached the backend; every refusal was the host's, before any call.
    assert_eq!(
        recorder.calls(),
        [
            format!("head {}", shown(readonly.path())),
            format!("init {}", shown(&project.path().join("fresh/repo"))),
        ]
    );
}
