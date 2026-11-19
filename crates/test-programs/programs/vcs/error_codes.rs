//! Every case a backend's failure reaches the guest as. Over the raw
//! bindings, each `VcsClient` error variant arrives as its `omnia:vcs` case.
//! Through the SDK, each case the SDK can reach lowers to its `vcs::Error`,
//! and that to an `omnia_sdk::Error` status and code. The SDK sends no
//! precondition and no log without a base, so it never reaches
//! `unsupported`. The suite's backend fails `descends` by the ancestor's
//! name.

#![cfg(target_arch = "wasm32")]

use omnia_sdk::WasiProvider;
use omnia_sdk::vcs::{Error as VcsError, Vcs as _};
use omnia_wasi_vcs::omnia::store;
use omnia_wasi_vcs::omnia::types::{Error, Location};
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
        ("exists", "invalid", "already exists: held"),
        ("not-found", "not-found", "not-found"),
        ("pending", "invalid", "pending changes: a.rs, b.rs"),
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
        ("exists", VcsError::Other("invalid: already exists: held".to_owned()), 500, "other"),
        ("access", VcsError::Access("refused".to_owned()), 502, "access"),
        ("other", VcsError::Other("broken".to_owned()), 500, "other"),
    ] {
        let lowered = WasiProvider.descends(".", fault, "sha:main").await;
        lowers(lowered.expect_err("the backend fails"), &error, status, code);
    }
    let denied = "denied: the workspace is beneath a read-only mount".to_owned();
    let refused = WasiProvider.commit("ro", "seal it").await.expect_err("a change beneath `ro`");
    lowers(refused, &VcsError::Other(denied), 500, "other");
    let absent = WasiProvider.head("./plain").await.expect_err("no workspace at `plain`");
    lowers(absent, &VcsError::NotARepository, 400, "not-a-repository");
    let diverged = WasiProvider.push(".", "moved", "main").await.expect_err("a moved bookmark");
    lowers(diverged, &VcsError::Diverged("main".to_owned()), 400, "diverged");
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
