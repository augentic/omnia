//! Mounts through the real runtime: each store preopens the directory a
//! mount opened at startup, and no mount may lie at or beneath a writable
//! one.

#![cfg(not(target_arch = "wasm32"))]

use std::fs;
use std::path::Path;

use anyhow::Result;
use omnia::{ExitStatus, GuestEntry, Mount, Runtime};
use omnia_test::host::{Backends, Deployment, scratch};
use omnia_wasi_otel::WasiOtel;

test_programs::foreach_mount!();

// After startup the host moves the mount's directory aside and puts another
// in its place, as a guest writing through an enclosing mount once could.
// The guest still reads the directory the mount opened.
#[tokio::test]
async fn mount_held() {
    let scratch = scratch();
    scratch.write("data/marker", "original");
    let data = scratch.path().join("data");

    let runtime = boot([mount("data", &data, true)], "original").await.expect("deployment boots");
    fs::rename(&data, scratch.path().join("moved")).expect("moving `data` aside");
    scratch.write("data/marker", "planted");

    assert_eq!(run(runtime).await, ExitStatus::SUCCESS, "the guest read the original marker");
}

// An operator's mount spelled through a symlink inside a writable mount is
// the directory the link led to at startup, which lies outside that mount.
// Relinking the path, as a guest writing through the writable mount could,
// moves nothing.
#[cfg(unix)]
#[tokio::test]
async fn symlinked_path() {
    use std::os::unix::fs::symlink;

    let scratch = scratch();
    scratch.write("release/marker", "original");
    scratch.write("other/marker", "planted");
    let writable = scratch.path().join("w");
    fs::create_dir(&writable).expect("creating `w`");
    let current = writable.join("current");
    symlink("../release", &current).expect("linking `current`");

    let runtime = boot([mount("w", &writable, true), mount("data", &current, false)], "original")
        .await
        .expect("the mount resolves outside `w`, so it boots");
    fs::remove_file(&current).expect("unlinking `current`");
    symlink("../other", &current).expect("relinking `current`");

    assert_eq!(run(runtime).await, ExitStatus::SUCCESS, "the guest read the original marker");
}

// A guest writing through a writable mount could rename, remove, or link
// anything beneath it, so a mount there is refused at assembly whatever its
// own permission, and so is a second name for the writable mount's
// directory.
#[tokio::test]
async fn beneath_writable() {
    let scratch = scratch();
    let inner = scratch.path().join("inner");
    fs::create_dir(&inner).expect("creating `inner`");

    for (nested, access) in [
        (mount("inner", &inner, false), "read-only"),
        (mount("inner", &inner, true), "writable"),
        (mount("inner", scratch.path(), false), "read-only"),
    ] {
        let mounts = [mount("outer", scratch.path(), true), nested];
        let Err(error) = boot(mounts, "original").await else {
            panic!("a mount at or beneath a writable mount is refused");
        };
        let detail = format!("{error:#}");
        assert!(detail.contains(&format!("the {access} mount `inner`")), "{detail}");
        assert!(detail.contains("beneath the writable mount `outer`"), "{detail}");
    }
}

// No guest can write through either mount, so read-only mounts nest.
#[tokio::test]
async fn nested_read_only() {
    let scratch = scratch();
    scratch.write("data/marker", "original");
    let mounts =
        [mount("outer", scratch.path(), false), mount("data", &scratch.path().join("data"), false)];

    let runtime = boot(mounts, "original").await.expect("read-only mounts may nest");
    assert_eq!(run(runtime).await, ExitStatus::SUCCESS, "the guest read the marker");
}

fn mount(name: &str, path: &Path, writable: bool) -> Mount {
    Mount {
        name: name.to_owned(),
        path: path.to_path_buf(),
        writable,
    }
}

// the `held` guest over `mounts`, expecting `contents` in `data/marker`
async fn boot(
    mounts: impl IntoIterator<Item = Mount>, contents: &str,
) -> Result<Runtime<Backends>> {
    Deployment::new()
        .mounts(mounts)
        .entry(GuestEntry::new("held", test_programs::MOUNT_HELD).command())
        .args([contents])
        .boot(Backends::defaults().await, |deployment| {
            deployment.host::<WasiOtel>()?;
            Ok(())
        })
        .await
}

async fn run(runtime: Runtime<Backends>) -> ExitStatus {
    let status = runtime.run_command().await;
    runtime.shutdown();
    status.expect("the guest exits")
}
