//! Entry planning for the generated `main`: macro-compiled [`MainOptions`]
//! plus process argv resolve into a deployment builder.
//!
//! Core plans only direct commands; the standard `run` grammar lives in
//! `omnia-cli`, which delegates the direct shape back here. [`plan`] is pure
//! with respect to the process — argv is a parameter — so argv policy is
//! unit-testable without spawning a binary.

use std::ffi::OsString;
use std::path::PathBuf;

use anyhow::{Result, anyhow};

use super::verbosity::{self, Levels};
use crate::{DeploymentBuilder, Manifest, Mode};

/// How a runtime's compiled-in deployment manifest is supplied.
///
/// The `runtime!` macro emits [`Path`](Self::Path) for its `manifest:` key
/// and [`Inline`](Self::Inline) for its inline manifest keys (`guests`,
/// `mounts`). On the standard CLI path (`omnia-cli`) it is the
/// lowest-priority source (behind `--manifest`/`OMNIA_MANIFEST` and a
/// positional wasm path); on the direct-command path it is the sole source.
#[derive(Clone, Debug)]
pub enum ManifestSource {
    /// A manifest path, loaded only when this source is selected.
    Path(PathBuf),
    /// A manifest value assembled at compile time.
    Inline(Manifest),
}

impl ManifestSource {
    /// Resolve into a manifest, loading the file for the path kind.
    ///
    /// # Errors
    ///
    /// Returns an error if a path source cannot be read or parsed.
    pub fn into_manifest(self) -> Result<Manifest> {
        match self {
            Self::Path(path) => Manifest::load(path),
            Self::Inline(manifest) => Ok(manifest),
        }
    }
}

/// Deployment options the `runtime!` macro compiles into the generated `main`.
#[doc(hidden)]
pub struct MainOptions {
    mode: Mode,
    manifest: Option<ManifestSource>,
    program_name: Option<String>,
}

impl MainOptions {
    /// Start options for a deployment driven in `mode`.
    #[must_use]
    pub const fn new(mode: Mode) -> Self {
        Self {
            mode,
            manifest: None,
            program_name: None,
        }
    }

    /// Set the compiled-in manifest source (the macro's `manifest:` key or
    /// inline manifest keys).
    #[must_use]
    pub fn manifest(mut self, source: ManifestSource) -> Self {
        self.manifest = Some(source);
        self
    }

    /// Set the program name (the macro passes the invoking crate's
    /// `CARGO_PKG_NAME`): telemetry's component name and, in command mode,
    /// the guest's `argv[0]`.
    #[must_use]
    pub fn program_name(mut self, name: impl Into<String>) -> Self {
        self.program_name = Some(name.into());
        self
    }

    /// Whether this is a direct command: command mode with a compiled-in manifest.
    #[must_use]
    pub fn is_direct(&self) -> bool {
        self.mode == Mode::Command && self.manifest.is_some()
    }

    /// Split into the mode, the compiled-in manifest source, and the program name.
    #[must_use]
    pub fn into_parts(self) -> (Mode, Option<ManifestSource>, Option<String>) {
        (self.mode, self.manifest, self.program_name)
    }
}

/// The planner's outcome: every deployment decision, resolved.
pub struct EntryPlan {
    mode: Mode,
    manifest: Option<Manifest>,
    program_name: Option<String>,
    args: Vec<String>,
    levels: Levels,
}

impl EntryPlan {
    /// Assemble the deployment builder this plan describes.
    pub fn into_builder(self) -> DeploymentBuilder {
        let builder =
            DeploymentBuilder::new().manifest(self.manifest).args(self.args).mode(self.mode);
        let builder = match self.program_name {
            Some(name) => builder.program_name(name),
            None => builder,
        };
        self.levels.apply(builder)
    }
}

/// Resolve [`MainOptions`] plus process argv into an [`EntryPlan`].
///
/// Command mode with a compiled-in manifest is a *direct command*: no host
/// CLI grammar, argv belongs to the guest verbatim. The direct plan always
/// carries the compiled-in manifest, so the builder never falls through to
/// its own `OMNIA_MANIFEST` lookup — the environment is untouched by design.
/// Every other shape needs the standard `run` grammar, which only
/// `omnia-cli` provides.
///
/// # Errors
///
/// Returns an error if the shape is not a direct command, or if the direct
/// plan fails (see [`plan_direct`]).
pub fn plan(options: MainOptions, argv: impl IntoIterator<Item = OsString>) -> Result<EntryPlan> {
    if options.is_direct() {
        return plan_direct(options, argv);
    }
    Err(anyhow!(
        "this runtime was built without omnia's `cli` feature; compile the deployment in \
         (command mode with a manifest) or enable the feature"
    ))
}

/// Plan a direct command: argv belongs to the guest verbatim, and the
/// compiled-in manifest is the sole source.
///
/// The host reads the verbosity flags (`-v`, `-q`, and their long forms) out
/// of argv to select the run's levels, but removes nothing: the guest
/// declares the same flags in its own grammar, so its help lists them and
/// its parser refuses `-v` with `-q`.
///
/// # Errors
///
/// Returns an error if argv is not UTF-8 or the compiled-in manifest cannot
/// be loaded.
pub fn plan_direct(
    options: MainOptions, argv: impl IntoIterator<Item = OsString>,
) -> Result<EntryPlan> {
    let (mode, manifest, program_name) = options.into_parts();
    let guest_args = argv
        .into_iter()
        .skip(1)
        .map(|arg| {
            arg.into_string()
                .map_err(|arg| anyhow!("guest argument `{}` is not valid UTF-8", arg.display()))
        })
        .collect::<Result<Vec<_>>>()?;
    let (verbose, quiet) = verbosity::scan(&guest_args);
    let levels = verbosity::levels(mode, verbose, quiet);
    let manifest = manifest.map(ManifestSource::into_manifest).transpose()?;
    Ok(EntryPlan {
        mode,
        manifest,
        program_name,
        args: guest_args,
        levels,
    })
}
