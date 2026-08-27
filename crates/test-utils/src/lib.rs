//! Shared harness for the e2e integration suites.
//!
//! The build script compiles every guest program in `crates/test-programs`
//! to a `wasm32-wasip2` component and generates one `pub const <NAME>: &str`
//! path per program plus a `foreach_<capability>!` macro; a suite invokes the
//! macro to prove every guest program has a matching test. The harness below
//! is capability-agnostic: a suite supplies its own backend bundle and links
//! its hosts in the `link` closure.

use std::sync::Arc;

use anyhow::Result;
use omnia::{Deployment, DeploymentBuilder, ExitStatus, Manifest, Mode, Mount, Runtime, StoreCtx};

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
    let manifest = Manifest::from_wasm(wasm).mounts(mounts);
    let mut deployment =
        DeploymentBuilder::new().manifest(manifest).mode(Mode::Command).build().await?;
    link(&mut deployment)?;

    let mounts = deployment.mounts();
    let args = deployment.args().to_vec();
    let registry = Arc::new(deployment.into_registry()?);
    Runtime::from_parts(registry, args, mounts, backends).run_command().await
}
