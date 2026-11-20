//! Every case a backend's failure reaches the guest as. Over the raw
//! bindings, each `VcsClient` error variant arrives as its `omnia:vcs` case.
//! Through the SDK, a failure is an `omnia_sdk::Error` under its case's
//! name, and an occupied place and a diverged push are the outcomes `init`
//! and `push` answer with. The suite's backend fails `descends` by the
//! ancestor's name.

#![cfg(target_arch = "wasm32")]

use omnia_sdk::vcs::types::{Error, Location};
use omnia_sdk::vcs::{self, store};
use wasip3::filesystem::preopens;

omnia_sdk::command::export!(scenario);

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
        ("denied", "denied", "refused"),
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

    // a failure and the outcomes through the SDK
    let absent = vcs::open("./plain").await.expect_err("no workspace at `plain`");
    assert_eq!((absent.code(), absent.description()), ("not-found", "not a repository"));
    let existing = vcs::init("./existing").await.expect("init");
    assert_eq!(existing.expect_err("a workspace already there").detail, "existing");
    let pushed = project.push("moved", "main").await.expect("push");
    let diverged = pushed.expect_err("a moved bookmark");
    assert_eq!(diverged.current.as_deref(), Some("rmt:moved:main"));
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
