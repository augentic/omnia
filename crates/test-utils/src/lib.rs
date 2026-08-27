//! Shared harness for the e2e integration suites.
//!
//! The build script compiles every guest program in `crates/test-programs`
//! to a `wasm32-wasip2` component and generates one `pub const <NAME>: &str`
//! path per program plus a `foreach_<capability>!` macro; a suite invokes the
//! macro to prove every guest program has a matching test. The harness below
//! is capability-agnostic: a suite supplies its own backend bundle and links
//! its hosts under test.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::{env, fs, process};

use anyhow::Result;
use omnia::{
    Deployment, DeploymentBuilder, ExitStatus, Host, Manifest, Mode, Mount, Runtime, Server,
    StoreCtx,
};

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

/// [`run_command`] for the common single-host suite: links `H` against the
/// bundle and runs.
///
/// # Errors
///
/// Same as [`run_command`].
pub async fn run_host<H, B>(wasm: &str, mounts: Vec<Mount>, backends: B) -> Result<ExitStatus>
where
    H: Host<StoreCtx<B>> + Server<B>,
    B: Clone + Send + Sync + 'static,
{
    run_command(wasm, mounts, backends, |deployment| {
        deployment.host::<H, B>()?;
        Ok(())
    })
    .await
}

/// A per-test scratch directory, removed on drop — including when the test
/// panics partway through.
pub struct Scratch(PathBuf);

impl Scratch {
    /// The directory path.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.0
    }

    /// A [`Mount`] preopening this directory into the guest sandbox as `.`.
    #[must_use]
    pub fn mount(&self, writable: bool) -> Mount {
        Mount {
            name: ".".to_owned(),
            path: self.0.clone(),
            writable,
        }
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Create a fresh [`Scratch`] directory; `tag` keeps concurrent tests apart.
///
/// # Panics
///
/// Panics if the directory cannot be created.
#[must_use]
pub fn scratch(tag: &str) -> Scratch {
    let dir = env::temp_dir().join(format!("omnia_scratch_{tag}_{}", process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("creating scratch dir");
    Scratch(dir)
}
