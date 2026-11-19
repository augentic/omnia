//! Raw bindings: what the host holds a `location` to before any backend
//! runs.
//!
//! - A root that is a file, or a directory the guest opened beneath its
//!   mount, is `denied`.
//! - A subpath that leaves the root, starts at `/`, or is not plainly
//!   spelled is `invalid`.
//! - A read-only mount opens a workspace, and refuses `init` and a change
//!   through the handle as `denied`.
//! - A name a file or a planted symlink holds is `invalid` for `init`, and
//!   a subpath that leads out of the mount is `denied`.
//! - A dangling or missing subpath is `not-found` for `open`, and `init`
//!   lays a missing one down.

#![cfg(target_arch = "wasm32")]

use omnia_sdk::vcs::store;
use omnia_sdk::vcs::types::{Error, Location};
use wasip3::filesystem::preopens;
use wasip3::filesystem::types::{Descriptor, DescriptorFlags, OpenFlags, PathFlags};

omnia_sdk::api::command::export!(scenario);

// a name beside the mount, through the planted `up`; the host checks it never appears
const OUTSIDE: &str = "up/omnia-vcs-locations-outside";

async fn scenario() {
    let directories = preopens::get_directories();
    let root = preopen(&directories, ".");
    let readonly = preopen(&directories, "ro");

    // a root the guest opened itself is not a mount root
    let nested = root
        .open_at(
            PathFlags::empty(),
            "nested".to_owned(),
            OpenFlags::DIRECTORY,
            DescriptorFlags::READ,
        )
        .await
        .expect("open nested dir");
    refused(store::open(at(&nested, "")).await.err(), "denied", "not an authorized mount");
    let file = root
        .open_at(
            PathFlags::empty(),
            "plain.txt".to_owned(),
            OpenFlags::CREATE,
            DescriptorFlags::READ | DescriptorFlags::WRITE,
        )
        .await
        .expect("create a file");
    refused(store::open(at(&file, "")).await.err(), "denied", "must be a directory");

    // a subpath that leaves the root, starts at it, or is not plainly spelled
    for subpath in ["../escape", "/abs", "a\\b", "a//b", ".", "..", "a/../b", "./a", "a/"]
        .into_iter()
        .map(str::to_owned)
        .chain(wasip3::cli::environment::get_arguments().into_iter().skip(1))
    {
        refused(
            store::open(at(root, &subpath)).await.err(),
            "invalid",
            "not a plain relative path",
        );
    }

    // a read-only mount opens a workspace and refuses every change
    let workspace = store::open(at(readonly, "")).await.expect("open beneath a read-only mount");
    assert_eq!(
        workspace.head().await.expect("read on a read-only mount"),
        Some("head-sha".to_owned())
    );
    refused(workspace.commit("seal it".to_owned()).await.err(), "denied", "read-only");
    refused(store::init(at(readonly, "")).await.err(), "denied", "read-only");
    refused(store::init(at(readonly, "fresh")).await.err(), "denied", "read-only");

    // a name a file or a planted symlink holds is no place to create, and a
    // subpath that leads out is the escape it is
    root.symlink_at("../../nowhere".to_owned(), "escape".to_owned())
        .await
        .expect("plant a symlink out");
    root.symlink_at("..".to_owned(), "up".to_owned()).await.expect("plant a symlink to the parent");
    root.symlink_at("nowhere".to_owned(), "dangling".to_owned())
        .await
        .expect("plant a dangling symlink");
    for subpath in ["plain.txt", "escape", "up", "dangling", "dangling/repo"] {
        refused(store::init(at(root, subpath)).await.err(), "invalid", "not a directory");
    }
    for subpath in ["escape/repo", OUTSIDE] {
        refused(store::init(at(root, subpath)).await.err(), "denied", "leads out");
    }
    for subpath in ["escape", "escape/repo", "up", OUTSIDE] {
        refused(store::open(at(root, subpath)).await.err(), "denied", "leads out");
    }
    for subpath in ["plain.txt", "dangling", "dangling/repo"] {
        refused(store::open(at(root, subpath)).await.err(), "not-found", "holds no directory");
    }

    // a subpath nothing holds yet is no workspace, and `init` lays it down
    refused(store::open(at(root, "fresh/repo")).await.err(), "not-found", "holds no directory");
    let fresh = store::init(at(root, "fresh/repo"))
        .await
        .expect("init at a new subpath")
        .expect("a new workspace");
    assert_eq!(
        fresh.head().await.expect("head of what init laid down"),
        Some("head-sha".to_owned())
    );
    store::open(at(root, "fresh/repo")).await.expect("open what init laid down");
}

fn preopen<'a>(directories: &'a [(Descriptor, String)], wanted: &str) -> &'a Descriptor {
    directories
        .iter()
        .find(|(_, name)| name == wanted)
        .map(|(dir, _)| dir)
        .unwrap_or_else(|| panic!("host mounts `{wanted}`"))
}

fn at<'a>(root: &'a Descriptor, subpath: &str) -> Location<'a> {
    Location {
        root,
        subpath: subpath.to_owned(),
    }
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
