//! Every case a backend's failure reaches the guest as. Over the raw
//! bindings, each `VcsClient` error variant arrives as its `omnia:vcs` case.
//! Through the SDK, each case the SDK can reach lowers to its `vcs::Error`,
//! and that to an `omnia_sdk::Error` status and code. An occupied place and
//! a merge into pending changes reach it as `Exists` and `Pending`. The SDK
//! sends no precondition and no log without a base, so it never reaches
//! `unsupported`. The suite's backend fails `descends` by the ancestor's
//! name.

#![cfg(target_arch = "wasm32")]

use omnia_sdk::vcs::types::{Error, Location};
use omnia_sdk::vcs::{self, Error as VcsError, store};
use wasip3::filesystem::preopens;

omnia_sdk::api::command::export!(scenario);

async fn scenario() {
    let directories = preopens::get_directories();
    let root = directories
        .iter()
        .find(|(_, name)| name == ".")
        .map(|(dir, _)| dir)
        .expect("host mounts `.`");

    // each backend failure over the raw bindings
    let project = store::open(Location {
        root,
        subpath: String::new(),
    })
    .await
    .expect("open the project");
    for (fault, case, detail) in [
        ("not-a-repository", "not-found", "not a repository"),
        ("exists", "other", "already exists: held"),
        ("not-found", "not-found", "not-found"),
        ("pending", "other", "pending changes: a.rs, b.rs"),
        ("access", "unavailable", "refused"),
        ("diverged", "other", "diverged: main"),
        ("other", "other", "broken"),
        ("untyped", "other", "fetching: network down"),
    ] {
        let error = project
            .is_ancestor(fault.to_owned(), "sha:main".to_owned())
            .await
            .expect_err("the backend fails");
        assert_eq!(case_of(&error), (case, detail), "from `{fault}`");
    }

    // each case the SDK reaches, as its error and its status
    for (fault, error, status, code) in [
        ("not-found", VcsError::NotFound("not-found".to_owned()), 404, "not-found"),
        ("exists", VcsError::Other("already exists: held".to_owned()), 500, "other"),
        ("access", VcsError::Access("refused".to_owned()), 502, "access"),
        ("other", VcsError::Other("broken".to_owned()), 500, "other"),
    ] {
        let lowered = vcs::descends(".", fault, "sha:main").await;
        lowers(lowered.expect_err("the backend fails"), &error, status, code);
    }
    let denied = "denied: the workspace is beneath a read-only mount".to_owned();
    let refused = vcs::commit("ro", "seal it").await.expect_err("a change beneath `ro`");
    lowers(refused, &VcsError::Other(denied), 500, "other");
    let absent = vcs::head("./plain").await.expect_err("no workspace at `plain`");
    lowers(absent, &VcsError::NotARepository, 400, "not-a-repository");
    let diverged = vcs::push(".", "moved", "main").await.expect_err("a moved bookmark");
    lowers(diverged, &VcsError::Diverged("main".to_owned()), 400, "diverged");
    let existing = vcs::init("./existing").await.expect_err("a repository already there");
    lowers(existing, &VcsError::Exists("existing".to_owned()), 400, "exists");
    let dirty = vcs::merge("./dirty", "feature", "merge it", &[]).await;
    let pending = VcsError::Pending(vec!["a.rs".to_owned(), "b.rs".to_owned(), "c.rs".to_owned()]);
    lowers(dirty.expect_err("a merge into pending changes"), &pending, 400, "pending");
}

fn case_of(error: &Error) -> (&'static str, &str) {
    match error {
        Error::NotFound(detail) => ("not-found", detail),
        Error::Denied(detail) => ("denied", detail),
        Error::Invalid(detail) => ("invalid", detail),
        Error::Unsupported(detail) => ("unsupported", detail),
        Error::Unavailable(detail) => ("unavailable", detail),
        Error::Other(detail) => ("other", detail),
    }
}

fn lowers(error: VcsError, expected: &VcsError, status: u16, code: &str) {
    assert_eq!(&error, expected);
    let error = omnia_sdk::Error::from(error);
    assert_eq!((error.status().as_u16(), error.code()), (status, code), "{error}");
}
