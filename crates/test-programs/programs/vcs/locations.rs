//! Raw bindings: what the host holds a `location` to before any backend
//! runs. A nested descriptor is not a mount root; a subpath leaves the root
//! or starts at `/`; a mutation beneath a read-only mount is refused where a
//! read is answered; a subpath that does not exist yet resolves for the
//! operation that creates it.

#![cfg(target_arch = "wasm32")]

use omnia_wasi_vcs::types::{Error, Location};
use omnia_wasi_vcs::{store, workspace};
use wasip3::filesystem::preopens;
use wasip3::filesystem::types::{Descriptor, DescriptorFlags, OpenFlags, PathFlags};

omnia_sdk::command!(scenario);

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

fn refused(error: Error, detail: &str) {
    assert!(
        matches!(error, Error::Other(ref message) if message.contains(detail)),
        "expected `{detail}`, got {error:?}"
    );
}

async fn scenario() {
    let directories = preopens::get_directories();
    let root = preopen(&directories, ".");
    let readonly = preopen(&directories, "ro");

    // a descriptor the guest opened beneath the mount is not the mount
    let nested = root
        .open_at(
            PathFlags::empty(),
            "nested".to_owned(),
            OpenFlags::DIRECTORY,
            DescriptorFlags::READ,
        )
        .await
        .expect("open nested dir");
    let error = store::head(at(&nested, "")).await.expect_err("nested descriptor refused");
    refused(error, "not an authorized mount");

    // a subpath that leaves the root, or starts at it
    let error = store::head(at(root, "../escape")).await.expect_err("escaping subpath refused");
    refused(error, "not a plain relative path");
    let error = store::head(at(root, "/abs")).await.expect_err("absolute subpath refused");
    refused(error, "not a plain relative path");

    // a read-only mount answers a read and refuses a mutation
    assert_eq!(store::head(at(readonly, "")).await.expect("read on a read-only mount"), "head-sha");
    let error = workspace::init(at(readonly, "")).await.expect_err("mutation refused");
    refused(error, "read-only");
    let error = workspace::init(at(readonly, "fresh")).await.expect_err("mutation beneath refused");
    refused(error, "read-only");

    // a subpath nothing holds yet resolves for the operation that creates it
    workspace::init(at(root, "fresh/repo")).await.expect("init at a new subpath");
}
