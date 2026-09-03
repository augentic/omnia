//! Shared harness for the e2e integration suites.
//!
//! The build script compiles every guest program in
//! `crates/test-programs/programs/<capability>/` to a `wasm32-wasip2`
//! component and generates one `pub const <NAME>: &str` path per program plus
//! a `foreach_<capability>!` macro; a suite invokes the macro to prove every
//! guest program has a matching test. The harness below is thin wrappers over
//! [`omnia_test::host`]: a suite supplies its own backend bundle and links its
//! hosts under test.

use std::path::Path;

use anyhow::Result;
use omnia::{Deployment, ExitStatus, Host, Mount, Provides, Server, StoreCtx};
pub use omnia_test::host::Scratch;
use omnia_wasi_otel::WasiOtel;

include!(concat!(env!("OUT_DIR"), "/gen.rs"));

/// Run `wasm` as a one-shot `wasi:cli` command deployment: `mounts` preopen
/// into the guest sandbox, `backends` becomes the store's backend bundle, and
/// `link` adds the hosts under test to the deployment.
///
/// # Errors
///
/// Returns an error if the deployment cannot be built or linked, or if the
/// guest traps without exiting.
pub async fn run_command<B>(
    wasm: &str, mounts: Vec<Mount>, backends: B,
    link: impl FnOnce(&mut Deployment<StoreCtx<B>>) -> Result<()>,
) -> Result<ExitStatus>
where
    B: Clone + Send + Sync + 'static,
{
    // The guest id follows `Manifest::from_wasm`: the artifact's file stem.
    let id = Path::new(wasm).file_stem().and_then(|stem| stem.to_str()).unwrap_or("default");
    omnia_test::host::Deployment::new().guest(id, wasm).mounts(mounts).run(backends, link).await
}

/// [`run_command`] for the common single-host suite: links `H` plus the
/// telemetry host every `omnia_guest::command!` guest imports, and runs.
///
/// # Errors
///
/// Same as [`run_command`].
pub async fn run_host<H, B>(wasm: &str, mounts: Vec<Mount>, backends: B) -> Result<ExitStatus>
where
    H: Host<StoreCtx<B>> + Server<B>,
    B: Provides<WasiOtel> + Clone + Send + Sync + 'static,
{
    run_command(wasm, mounts, backends, |deployment| {
        deployment.host::<H, B>()?;
        deployment.host::<WasiOtel, B>()?;
        Ok(())
    })
    .await
}

/// A fresh [`Scratch`] directory; the tempdir is unique on its own, so `tag`
/// is kept only for the call sites' readability.
///
/// # Panics
///
/// Panics if the directory cannot be created.
#[must_use]
pub fn scratch(_tag: &str) -> Scratch {
    omnia_test::host::scratch()
}
