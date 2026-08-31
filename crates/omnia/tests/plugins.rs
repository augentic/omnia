//! End-to-end tests for the `omnia:plugins/loader` host capability: a real
//! requester guest from `crates/test-programs` drives loads through omnia's
//! runtime, with `MountAcquire` reading components staged in a scratch mount.
//! The requester asserts internally (handles, digests, dispatch answers, and
//! every typed refusal); the host side stages artifacts and checks the exit.

#![cfg(not(target_arch = "wasm32"))]

use anyhow::{Context as _, Result};
use omnia::{
    DeploymentBuilder, ExitStatus, GuestEntry, Manifest, Mode, MountAcquire, Runtime, StoreCtx,
    WasiPlugins,
};

// Every guest program in `crates/test-programs/programs/plugins` must have a
// matching test here; a new program without one fails to compile.
test_utils::foreach_plugins!();

/// Drive `wasm` as the `requester` command guest: the scratch dir mounts at
/// `.`, `omnia-test:link/ops` is the declared plugin seam, and `MountAcquire`
/// is the compiled-in acquirer.
async fn run_requester(wasm: &str, scratch: &test_utils::Scratch) -> Result<ExitStatus> {
    let manifest = Manifest::new()
        .plugins(["omnia-test:link/ops"])
        .guest(GuestEntry::new("requester", wasm))
        .mounts([scratch.mount(false)]);
    let deployment = DeploymentBuilder::new()
        .manifest(manifest)
        .mode(Mode::Command)
        .build::<StoreCtx<()>>()
        .await
        .context("building deployment")?;
    let runtime = Runtime::new(
        deployment,
        |deployment| {
            deployment.host::<WasiPlugins, ()>()?;
            Ok(())
        },
        |()| Some(std::sync::Arc::new(MountAcquire)),
    )
    .await
    .context("assembling runtime")?;
    runtime.run_command().await
}

#[tokio::test]
async fn plugins_load_path() {
    let scratch = test_utils::scratch("plugins_load_path");
    std::fs::copy(test_utils::LINK_ECHOER, scratch.path().join("plugin.wasm"))
        .expect("staging the loadable echoer");

    let status =
        run_requester(test_utils::PLUGINS_LOAD_PATH, &scratch).await.expect("deployment runs");
    assert_eq!(status, ExitStatus::SUCCESS, "the requester's assertions all held");
}

// The guest-side copies exist because published crates cannot reference WIT
// outside their package root; the host copy stays canonical.
#[test]
fn wit_copies_stay_identical() {
    let canonical = include_str!("../wit/plugins.wit");
    assert_eq!(
        include_str!("../../omnia-guest/wit/plugins.wit"),
        canonical,
        "omnia-guest's plugins.wit copy drifted from crates/omnia/wit/plugins.wit"
    );
    assert_eq!(
        include_str!("../../test-programs/wit/deps/plugins/plugins.wit"),
        canonical,
        "test-programs' plugins.wit copy drifted from crates/omnia/wit/plugins.wit"
    );
}

// Compile-time proof that the macro's `locations:`/`cache:` grammar lowers
// into calls that typecheck against this crate's public seam: path entries
// fold into a `PathAcquire`, the registry entry into a `RegistryAcquire`
// cached in the `cache:` backend, composed in the `Wiring::acquirer` hook.
// Never run — the macro's snapshot suite pins the shape, this pins the types.
mod locations_grammar {
    use std::future::Future;

    use anyhow::Result;
    use omnia::futures::future::BoxFuture;
    use omnia::{Backend, NoOptions, NoStore, PluginStore, ReleaseRecord};

    // Clone without Copy: the generated hook clones the backend out of the
    // bundle, and a Copy type would trip `clippy::clone_on_copy` there.
    #[derive(Clone)]
    struct Cache;

    impl Backend for Cache {
        type ConnectOptions = NoOptions;

        fn connect_with(_options: NoOptions) -> impl Future<Output = Result<Self>> {
            std::future::ready(Ok(Self))
        }
    }

    impl PluginStore for Cache {
        fn get_content<'a>(&'a self, digest: &'a str) -> BoxFuture<'a, Result<Option<Vec<u8>>>> {
            NoStore.get_content(digest)
        }

        fn put_content<'a>(
            &'a self, digest: &'a str, bytes: &'a [u8],
        ) -> BoxFuture<'a, Result<()>> {
            NoStore.put_content(digest, bytes)
        }

        fn get_release<'a>(
            &'a self, registry: &'a str, package: &'a str, version: &'a str,
        ) -> BoxFuture<'a, Result<Option<ReleaseRecord>>> {
            NoStore.get_release(registry, package, version)
        }

        fn put_release<'a>(
            &'a self, registry: &'a str, package: &'a str, record: &'a ReleaseRecord,
        ) -> BoxFuture<'a, Result<()>> {
            NoStore.put_release(registry, package, record)
        }
    }

    omnia::runtime!({
        plugins: {
            interfaces: ["omnia-test:link/ops"],
            locations: [
                { name: "adapters", path: "adapters" },
                { registry: "ghcr.io" },
            ],
            cache: Cache,
        },
        guests: [
            { id: "engine", source: "engine.wasm" },
        ],
    });
}

#[test]
fn locations_grammar_expands() {
    // Touch the generated entry points so the compile-only module above is
    // reachable for dead-code analysis.
    let _ = (locations_grammar::main, locations_grammar::run);
}

#[tokio::test]
async fn plugins_load_refused() {
    let scratch = test_utils::scratch("plugins_load_refused");
    std::fs::copy(test_utils::LINK_ECHOER, scratch.path().join("plugin.wasm"))
        .expect("staging the loadable echoer");
    // Exports no `omnia-test:link/ops` instance — the seam-missing target.
    std::fs::copy(test_utils::LINK_FULL, scratch.path().join("noseam.wasm"))
        .expect("staging the seamless component");
    // Leading ELF magic is exactly what the loader sniffs; the tail is junk,
    // proving refusal happens before any wasmtime parsing.
    std::fs::write(scratch.path().join("native.bin"), [0x7f, b'E', b'L', b'F', 0, 0, 0, 0])
        .expect("staging native bytes");

    let status =
        run_requester(test_utils::PLUGINS_LOAD_REFUSED, &scratch).await.expect("deployment runs");
    assert_eq!(status, ExitStatus::SUCCESS, "the requester's assertions all held");
}
