//! End-to-end tests for `omnia:vcs`: every scenario runs a guest component
//! from `crates/test-programs` through the omnia runtime over a recording
//! backend. The guest asserts what crosses the boundary and traps on
//! failure; the host asserts the place every location resolved to and the
//! calls the backend saw, in order, so a call the host refused itself is
//! one the backend never saw.

#![cfg(not(target_arch = "wasm32"))]

use std::fs;
use std::path::Path;
use std::sync::Arc;

use anyhow::anyhow;
use cap_std::ambient_authority;
use cap_std::fs::{Dir, MetadataExt as _};
use futures::FutureExt as _;
use omnia::{ExitStatus, Mount};
use omnia_test::host::{Backends, Deployment, scratch};
use omnia_wasi_vcs::{
    Change, ChangeKind, CloneOptions, Entry, Error, FutureResult, Merged, Place, Rule, VcsClient,
    WasiVcs,
};
use parking_lot::Mutex;

// Every guest program in `crates/test-programs` must have a matching test
// here; a new program without one fails to compile.
test_programs::foreach_vcs!();

// ------------------------------------------------------------------------
// Harness
// ------------------------------------------------------------------------

async fn run_guest(wasm: &str, mounts: Vec<Mount>) -> Recorder {
    run_guest_args(wasm, mounts, &[]).await
}

async fn run_guest_args(wasm: &str, mounts: Vec<Mount>, args: &[&str]) -> Recorder {
    let recorder = Recorder::default();
    let backends = Backends::defaults().await.vcs(recorder.clone());
    let status = Deployment::new()
        .guest("guest", wasm)
        .mounts(mounts)
        .args(args.iter().copied())
        .run_host::<WasiVcs, _>(backends)
        .await
        .expect("guest runs");
    assert_eq!(status, ExitStatus::SUCCESS, "guest `{wasm}` failed");
    recorder
}

// ------------------------------------------------------------------------
// Scenario backend
// ------------------------------------------------------------------------

// The entries in the long log and the long status, one more than a page
// holds.
const LONG: usize = 1001;

// Records every call with the host path its place resolved to, and answers
// from a fixed script keyed on the place's last segment and on the names a
// call carries:
//
// - `head` at `plain` is no repository, and at `unborn` finds no commit;
// - `init` at `existing` finds a repository there already;
// - `pending` at `dirty` is three changes and at `big` is `LONG`, both out
//   of path order, and elsewhere is none;
// - a log of `long-<n>` runs from `n` to the end of a `LONG`-commit chain,
//   and any other log is two commits;
// - `resolve`, `labelled`, and `fetched` of `missing` find nothing, and
//   `fetched` from the remote `unknown` names that remote;
// - a merge of `conflicting` comes back in conflict, out of path order;
// - `push` to `moved` diverges;
// - `descends` from `unrelated` is false, and from a fault's name fails
//   with that fault.
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

// The place's path is informational; the handle it holds must be the very
// directory that path names, so a backend working through it works there.
fn held(place: &Place) -> String {
    let by_handle = place.dir().dir_metadata().expect("place handle metadata");
    let by_path = Dir::open_ambient_dir(place.path(), ambient_authority())
        .and_then(|dir| dir.dir_metadata())
        .expect("place path opens");
    assert_eq!(
        (by_handle.dev(), by_handle.ino()),
        (by_path.dev(), by_path.ino()),
        "place handle is the directory at `{}`",
        place.path().display()
    );
    shown(place.path())
}

fn named(place: &Place) -> String {
    place.path().file_name().map_or_else(String::new, |name| name.to_string_lossy().into_owned())
}

// Every variant of the seam's `Error`, and an untyped failure with context.
fn fault(name: &str) -> Option<anyhow::Error> {
    let error = match name {
        "not-a-repository" => Error::NotARepository,
        "exists" => Error::Exists("held".to_owned()),
        "not-found" => Error::NotFound("not-found".to_owned()),
        "pending" => Error::Pending(vec!["a.rs".to_owned(), "b.rs".to_owned()]),
        "access" => Error::Access("refused".to_owned()),
        "diverged" => Error::Diverged("main".to_owned()),
        "other" => Error::Other("broken".to_owned()),
        "untyped" => return Some(anyhow!("network down").context("fetching")),
        _ => return None,
    };
    Some(error.into())
}

fn change(path: &str, kind: ChangeKind) -> Change {
    Change {
        path: path.to_owned(),
        kind,
    }
}

impl VcsClient for Recorder {
    fn resolve(&self, repo: Place, revision: String) -> FutureResult<String> {
        self.record(format!("resolve {} {revision}", held(&repo)));
        async move {
            if revision == "missing" {
                return Err(Error::NotFound(revision).into());
            }
            Ok(format!("sha:{revision}"))
        }
        .boxed()
    }

    fn descends(&self, repo: Place, ancestor: String, descendant: String) -> FutureResult<bool> {
        self.record(format!("descends {} {ancestor} {descendant}", held(&repo)));
        async move {
            if let Some(fault) = fault(&ancestor) {
                return Err(fault);
            }
            Ok(ancestor != "unrelated")
        }
        .boxed()
    }

    fn head(&self, at: Place) -> FutureResult<String> {
        self.record(format!("head {}", held(&at)));
        let name = named(&at);
        async move {
            match name.as_str() {
                "plain" => Err(Error::NotARepository.into()),
                "unborn" => Err(Error::NotFound("HEAD".to_owned()).into()),
                _ => Ok("head-sha".to_owned()),
            }
        }
        .boxed()
    }

    fn commit(&self, at: Place, message: String) -> FutureResult<Option<String>> {
        self.record(format!("commit {} {message:?}", held(&at)));
        async move { Ok(Some(format!("c:{message}"))) }.boxed()
    }

    fn merge(
        &self, at: Place, revision: String, message: String, policy: Vec<Rule>,
    ) -> FutureResult<Merged> {
        let rules: Vec<String> =
            policy.iter().map(|rule| format!("{}={:?}", rule.paths, rule.strategy)).collect();
        self.record(format!("merge {} {revision} {message:?} [{}]", held(&at), rules.join(",")));
        async move {
            if revision == "conflicting" {
                return Ok(Merged {
                    commit: None,
                    conflicts: vec!["z.lock".to_owned(), "Cargo.lock".to_owned()],
                });
            }
            Ok(Merged {
                commit: Some(format!("m:{revision}")),
                conflicts: vec![],
            })
        }
        .boxed()
    }

    fn log(&self, repo: Place, revision: String, base: String) -> FutureResult<Vec<Entry>> {
        self.record(format!("log {} {revision} {base}", held(&repo)));
        async move {
            if let Some(from) = revision.strip_prefix("long-") {
                let from: usize = from.parse()?;
                let chain = (from..LONG).map(|n| Entry {
                    id: format!("long-{n}"),
                    message: format!("commit {n}"),
                });
                return Ok(chain.collect());
            }
            Ok(vec![
                Entry {
                    id: format!("l2:{revision}"),
                    message: "second\n\nSlice: SLICE-002\n".to_owned(),
                },
                Entry {
                    id: format!("l1:{revision}"),
                    message: "first\n\nSlice: SLICE-001\n".to_owned(),
                },
            ])
        }
        .boxed()
    }

    fn init(&self, at: Place) -> FutureResult<()> {
        let path = held(&at);
        self.record(format!("init {path}"));
        let name = named(&at);
        async move {
            if name == "existing" {
                return Err(Error::Exists(path).into());
            }
            Ok(())
        }
        .boxed()
    }

    fn add(&self, repo: Place, at: Place, revision: String) -> FutureResult<()> {
        self.record(format!("add {} {} {revision}", held(&repo), held(&at)));
        async { Ok(()) }.boxed()
    }

    fn remove(&self, at: Place) -> FutureResult<()> {
        self.record(format!("remove {}", held(&at)));
        async { Ok(()) }.boxed()
    }

    fn pending(&self, at: Place) -> FutureResult<Vec<Change>> {
        self.record(format!("pending {}", held(&at)));
        let name = named(&at);
        async move {
            Ok(match name.as_str() {
                "dirty" => vec![
                    change("c.rs", ChangeKind::Deleted),
                    change("a.rs", ChangeKind::Added),
                    change("b.rs", ChangeKind::Modified),
                ],
                "big" => (0..LONG)
                    .rev()
                    .map(|n| change(&format!("f{n:04}.rs"), ChangeKind::Added))
                    .collect(),
                _ => vec![],
            })
        }
        .boxed()
    }

    fn clone_repo(&self, url: String, at: Place, options: CloneOptions) -> FutureResult<()> {
        self.record(format!("clone {url} {} depth={:?}", held(&at), options.depth));
        async { Ok(()) }.boxed()
    }

    fn fetch(&self, repo: Place, remote: String) -> FutureResult<()> {
        self.record(format!("fetch {} {remote}", held(&repo)));
        async { Ok(()) }.boxed()
    }

    fn label(&self, repo: Place, name: String, revision: String) -> FutureResult<()> {
        self.record(format!("label {} {name} {revision}", held(&repo)));
        async { Ok(()) }.boxed()
    }

    fn labelled(&self, repo: Place, name: String) -> FutureResult<String> {
        self.record(format!("labelled {} {name}", held(&repo)));
        async move {
            if name == "missing" {
                return Err(Error::NotFound(name).into());
            }
            Ok(format!("lbl:{name}"))
        }
        .boxed()
    }

    fn fetched(&self, repo: Place, remote: String, name: String) -> FutureResult<String> {
        self.record(format!("fetched {} {remote} {name}", held(&repo)));
        async move {
            if remote == "unknown" {
                return Err(Error::NotFound(remote).into());
            }
            if name == "missing" {
                return Err(Error::NotFound(name).into());
            }
            Ok(format!("rmt:{remote}:{name}"))
        }
        .boxed()
    }

    fn push(&self, repo: Place, remote: String, label: String) -> FutureResult<()> {
        self.record(format!("push {} {remote} {label}", held(&repo)));
        async move {
            if remote == "moved" {
                return Err(Error::Diverged(label).into());
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
    for dir in ["work", "dirty", "unborn", "big"] {
        fs::create_dir(project.path().join(dir)).expect("creating a working copy");
    }
    let workspaces = scratch();
    fs::create_dir(workspaces.path().join("repo")).expect("creating the repository under `ws`");
    let nested = scratch();
    fs::create_dir(nested.path().join("repo")).expect("creating the repository under `ws/sub`");

    let recorder = run_guest(
        test_programs::VCS_FLOW,
        vec![
            project.mount(true),
            workspaces.mount_as("ws", false),
            nested.mount_as("ws/sub", false),
        ],
    )
    .await;

    // Every call opened its workspace with a `head` first, but `init` and
    // `clone_repo`, which lay their place down. A path no preopen answers
    // reached nothing.
    for created in ["fresh", "existing", ".cache/work", "clone"] {
        assert!(project.path().join(created).is_dir(), "host created `{created}`");
    }
    let root = shown(project.path());
    let [work, dirty, unborn, big, fresh, existing, cache, clone] =
        ["work", "dirty", "unborn", "big", "fresh", "existing", ".cache/work", "clone"]
            .map(|subpath| shown(&project.path().join(subpath)));
    let (ws, sub) = (shown(&workspaces.path().join("repo")), shown(&nested.path().join("repo")));
    assert_eq!(
        recorder.calls(),
        [
            format!("head {root}"),
            format!("resolve {root} main"),
            format!("head {root}"),
            format!("descends {root} sha:base sha:main"),
            format!("head {root}"),
            format!("descends {root} unrelated sha:main"),
            format!("head {root}"),
            format!("log {root} emery/rev sha:base"),
            format!("head {work}"),
            format!("head {work}"),
            format!("head {work}"),
            format!("commit {work} \"seal it\""),
            format!("head {work}"),
            format!("pending {work}"),
            format!("merge {work} feature \"merge it\" [*.lock=Ours]"),
            format!("head {work}"),
            format!("pending {work}"),
            format!("merge {work} conflicting \"merge it\" []"),
            format!("head {dirty}"),
            format!("pending {dirty}"),
            format!("head {unborn}"),
            format!("head {unborn}"),
            format!("init {fresh}"),
            format!("init {existing}"),
            format!("head {root}"),
            format!("add {root} {cache} sha:main"),
            format!("head {cache}"),
            format!("remove {cache}"),
            format!("clone https://example.test/repo.git {clone} depth=Some(1)"),
            format!("head {clone}"),
            format!("fetch {clone} origin"),
            format!("head {clone}"),
            format!("label {clone} emery/rev sha:main"),
            format!("head {clone}"),
            format!("labelled {clone} emery/rev"),
            format!("head {clone}"),
            format!("fetched {clone} origin emery/rev"),
            format!("head {clone}"),
            format!("push {clone} origin emery/rev"),
            format!("head {clone}"),
            format!("push {clone} moved emery/rev"),
            format!("head {root}"),
            format!("resolve {root} missing"),
            format!("head {clone}"),
            format!("labelled {clone} missing"),
            format!("head {clone}"),
            format!("fetched {clone} origin missing"),
            format!("head {root}"),
            format!("log {root} long-0 sha:base"),
            format!("log {root} long-1000 sha:base"),
            format!("head {big}"),
            format!("pending {big}"),
            format!("pending {big}"),
            format!("head {ws}"),
            format!("head {ws}"),
            format!("head {sub}"),
            format!("head {sub}"),
        ]
    );
}

#[tokio::test]
async fn vcs_workspace() {
    let project = scratch();
    for dir in ["plain", "unborn", "dirty", "work"] {
        fs::create_dir(project.path().join(dir)).expect("creating a working copy");
    }

    let recorder = run_guest(test_programs::VCS_WORKSPACE, vec![project.mount(true)]).await;

    // The refusals the host makes itself reached no backend call: a zero
    // `limit`, a log with no base, a precondition, a removed handle, and a
    // clone of no depth. A merge into pending changes reached only the
    // `pending` that found them.
    let root = shown(project.path());
    let [fresh, plain, unborn, dirty, work, cache, clone] =
        ["fresh", "plain", "unborn", "dirty", "work", ".cache/work", "clone"]
            .map(|subpath| shown(&project.path().join(subpath)));
    assert_eq!(
        recorder.calls(),
        [
            format!("head {root}"),
            format!("init {fresh}"),
            format!("head {fresh}"),
            format!("head {plain}"),
            format!("resolve {root} main"),
            format!("resolve {root} missing"),
            format!("descends {root} sha:base sha:main"),
            format!("labelled {root} emery/rev"),
            format!("labelled {root} missing"),
            format!("head {unborn}"),
            format!("head {unborn}"),
            format!("log {root} long-999 sha:base"),
            format!("log {root} long-1000 sha:base"),
            format!("head {dirty}"),
            format!("pending {dirty}"),
            format!("pending {dirty}"),
            format!("pending {dirty}"),
            format!("label {root} emery/rev sha:main"),
            format!("head {work}"),
            format!("commit {work} \"seal it\""),
            format!("pending {work}"),
            format!("merge {work} feature \"merge it\" [*.lock=Ours]"),
            format!("pending {work}"),
            format!("merge {work} conflicting \"merge it\" []"),
            format!("pending {dirty}"),
            format!("add {root} {cache} sha:main"),
            format!("head {cache}"),
            format!("remove {cache}"),
            format!("clone https://example.test/repo.git {clone} depth=Some(1)"),
            format!("fetch {clone} origin"),
            format!("fetched {clone} origin emery/rev"),
            format!("fetched {clone} origin missing"),
            format!("fetched {clone} unknown emery/rev"),
            format!("push {clone} origin emery/rev"),
            format!("push {clone} moved emery/rev"),
        ]
    );
}

#[tokio::test]
async fn vcs_locations() {
    // the mount sits one level down, so whatever escapes through `up` lands
    // in this test's own scratch rather than the shared temp directory
    let outside = scratch();
    let project = outside.path().join("project");
    fs::create_dir_all(project.join("nested")).expect("creating nested dir");
    let readonly = scratch();

    let mount = Mount {
        name: ".".to_owned(),
        path: project.clone(),
        writable: true,
    };
    let extra: &[&str] = if cfg!(windows) { &["C:evil", "C:/evil"] } else { &[] };
    let recorder = run_guest_args(
        test_programs::VCS_LOCATIONS,
        vec![mount, readonly.mount_as("ro", false)],
        extra,
    )
    .await;

    // Only the workspace on the read-only mount and the one at the new
    // subpath reached the backend; every refusal was the host's, before any
    // call.
    let (ro, fresh) = (shown(readonly.path()), shown(&project.join("fresh/repo")));
    assert_eq!(
        recorder.calls(),
        [
            format!("head {ro}"),
            format!("head {ro}"),
            format!("init {fresh}"),
            format!("head {fresh}"),
            format!("head {fresh}"),
        ]
    );
    for link in ["escape", "up", "dangling"] {
        assert!(
            fs::symlink_metadata(project.join(link)).is_ok_and(|meta| meta.is_symlink()),
            "the guest planted `{link}` through the mount"
        );
    }
    assert!(!project.join("nowhere").exists(), "no link target was materialized");
    assert!(
        !outside.path().join("omnia-vcs-locations-outside").exists(),
        "nothing was laid down through `up`"
    );
    assert!(!readonly.path().join("fresh").exists(), "nothing was laid down beneath `ro`");
}

#[tokio::test]
async fn vcs_error_codes() {
    let project = scratch();
    fs::create_dir(project.path().join("plain")).expect("creating a directory with no workspace");
    let readonly = scratch();

    let recorder = run_guest(
        test_programs::VCS_ERROR_CODES,
        vec![project.mount(true), readonly.mount_as("ro", false)],
    )
    .await;

    // The commit beneath `ro` reached the backend only as the `head` that
    // opened its workspace.
    let root = shown(project.path());
    let (ro, plain) = (shown(readonly.path()), shown(&project.path().join("plain")));
    assert_eq!(
        recorder.calls(),
        [
            format!("head {root}"),
            format!("descends {root} not-a-repository sha:main"),
            format!("descends {root} exists sha:main"),
            format!("descends {root} not-found sha:main"),
            format!("descends {root} pending sha:main"),
            format!("descends {root} access sha:main"),
            format!("descends {root} diverged sha:main"),
            format!("descends {root} other sha:main"),
            format!("descends {root} untyped sha:main"),
            format!("head {root}"),
            format!("descends {root} not-found sha:main"),
            format!("head {root}"),
            format!("descends {root} exists sha:main"),
            format!("head {root}"),
            format!("descends {root} access sha:main"),
            format!("head {root}"),
            format!("descends {root} other sha:main"),
            format!("head {ro}"),
            format!("head {plain}"),
            format!("head {root}"),
            format!("push {root} moved main"),
        ]
    );
}
