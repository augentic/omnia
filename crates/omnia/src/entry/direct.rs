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

use super::verbosity;
use crate::{DeploymentBuilder, LevelFilter, Manifest, Mode};

/// How a runtime's compiled-in deployment manifest is supplied.
///
/// The `runtime!` macro emits [`Path`](Self::Path) for its `config:` key and
/// [`Inline`](Self::Inline) for its inline manifest keys (`guests`,
/// `mounts`). On the standard CLI path (`omnia-cli`) it is the
/// lowest-priority source (behind `--config`/`OMNIA_CONFIG` and a positional
/// wasm path); on the direct-command path it is the sole source.
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
            Self::Path(path) => Manifest::from_config(path),
            Self::Inline(manifest) => Ok(manifest),
        }
    }
}

/// Deployment options the `runtime!` macro compiles into the generated `main`.
#[doc(hidden)]
pub struct MainOptions {
    mode: Mode,
    manifest: Option<ManifestSource>,
}

impl MainOptions {
    /// Start options for a deployment driven in `mode`.
    #[must_use]
    pub const fn new(mode: Mode) -> Self {
        Self { mode, manifest: None }
    }

    /// Set the compiled-in manifest source (the macro's `config:` key or
    /// inline manifest keys).
    #[must_use]
    pub fn manifest(mut self, source: ManifestSource) -> Self {
        self.manifest = Some(source);
        self
    }

    /// Whether this is a direct command: command mode with a compiled-in manifest.
    #[must_use]
    pub fn is_direct(&self) -> bool {
        self.mode == Mode::Command && self.manifest.is_some()
    }

    /// Split into the mode and the compiled-in manifest source.
    #[must_use]
    pub fn into_parts(self) -> (Mode, Option<ManifestSource>) {
        (self.mode, self.manifest)
    }
}

/// The planner's outcome: every deployment decision, resolved.
pub(super) struct EntryPlan {
    mode: Mode,
    manifest: Option<Manifest>,
    args: Vec<String>,
    level: Option<LevelFilter>,
}

impl EntryPlan {
    /// Assemble the deployment builder this plan describes.
    pub(super) fn into_builder(self) -> DeploymentBuilder {
        let builder =
            DeploymentBuilder::new().manifest(self.manifest).args(self.args).mode(self.mode);
        match self.level {
            Some(level) => builder.level(level),
            None => builder,
        }
    }
}

/// Resolve [`MainOptions`] plus process argv into an [`EntryPlan`].
///
/// Command mode with a compiled-in manifest is a *direct command*: no host
/// CLI grammar, argv belongs to the guest verbatim. The direct plan always
/// carries the compiled-in manifest, so the builder never falls through to
/// its own `OMNIA_CONFIG` lookup — the environment is untouched by design.
/// Every other shape needs the standard `run` grammar, which only
/// `omnia-cli` provides.
///
/// # Errors
///
/// Returns an error if the shape is not a direct command, or if the direct
/// plan fails (see [`plan_direct`]).
pub(super) fn plan(
    options: MainOptions, argv: impl IntoIterator<Item = OsString>,
) -> Result<EntryPlan> {
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
/// of argv to select the process level, but removes nothing: the guest
/// declares the same flags in its own grammar, so its help lists them and
/// its parser refuses `-v` with `-q`.
///
/// # Errors
///
/// Returns an error if argv is not UTF-8 or the compiled-in manifest cannot
/// be loaded.
pub(super) fn plan_direct(
    options: MainOptions, argv: impl IntoIterator<Item = OsString>,
) -> Result<EntryPlan> {
    let (mode, manifest) = options.into_parts();
    let guest_args = argv
        .into_iter()
        .skip(1)
        .map(|arg| {
            arg.into_string()
                .map_err(|arg| anyhow!("guest argument `{}` is not valid UTF-8", arg.display()))
        })
        .collect::<Result<Vec<_>>>()?;
    let (verbose, quiet) = verbosity::scan(&guest_args);
    let level = verbosity::level(mode.level(), verbose, quiet);
    let manifest = manifest.map(ManifestSource::into_manifest).transpose()?;
    Ok(EntryPlan {
        mode,
        manifest,
        args: guest_args,
        level,
    })
}

// Unit tests by design: `plan` is factored pure (argv is a parameter)
// precisely so argv policy is testable without spawning a binary.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::GuestEntry;

    fn argv(args: &[&str]) -> Vec<OsString> {
        args.iter().map(OsString::from).collect()
    }

    fn inline_source(guest: &str) -> ManifestSource {
        ManifestSource::Inline(
            Manifest::new().guest(GuestEntry::new(guest, format!("{guest}.wasm"))),
        )
    }

    fn first_guest(plan: &EntryPlan) -> &str {
        plan.manifest.as_ref().expect("plan carries a manifest").guests[0].id.as_str()
    }

    fn fatal(error: &anyhow::Error) -> String {
        format!("{error:#}")
    }

    #[test]
    fn direct_argv() {
        // `--config`, `run`, and `--debug` are guest arguments, not host CLI
        // options: nothing in argv is reserved for the host.
        let options = MainOptions::new(Mode::Command).manifest(inline_source("app"));
        let plan = plan(options, argv(&["bin", "--config", "foo.toml", "run", "--debug", "greet"]))
            .unwrap_or_else(|error| panic!("{}", fatal(&error)));
        assert_eq!(plan.args, ["--config", "foo.toml", "run", "--debug", "greet"]);
        assert_eq!(first_guest(&plan), "app");
        assert_eq!(plan.level, None, "no flag leaves the process `RUST_LOG` standing");
    }

    // The verbosity flags are read for the level and stay in the guest's
    // argv, where its own grammar declares them.
    #[test]
    fn direct_argv_verbatim() {
        let options = MainOptions::new(Mode::Command).manifest(inline_source("app"));
        let verbose = plan(options, argv(&["bin", "-v", "greet"]))
            .unwrap_or_else(|error| panic!("{}", fatal(&error)));
        assert_eq!(verbose.args, ["-v", "greet"]);
        assert_eq!(verbose.level, Some(LevelFilter::DEBUG), "one rung up from command `info`");

        let options = MainOptions::new(Mode::Command).manifest(inline_source("app"));
        let conflict = plan(options, argv(&["bin", "-v", "greet", "--quiet"]))
            .unwrap_or_else(|error| panic!("{}", fatal(&error)));
        assert_eq!(conflict.args, ["-v", "greet", "--quiet"]);
        assert_eq!(conflict.level, None, "the guest refuses the pair; the host applies nothing");
    }

    // Hard acceptance criterion: the direct plan always carries the compiled-in
    // manifest, so `DeploymentBuilder::build` can never fall through to its own
    // `OMNIA_CONFIG` lookup.
    #[test]
    fn direct_compiled_manifest() {
        let options = MainOptions::new(Mode::Command).manifest(inline_source("app"));
        let plan = plan(options, argv(&["bin", "greet"]))
            .unwrap_or_else(|error| panic!("{}", fatal(&error)));
        assert_eq!(first_guest(&plan), "app", "the compiled-in manifest is the sole source");
        assert_eq!(plan.args, ["greet"]);
    }

    #[test]
    fn non_direct() {
        let error = plan(MainOptions::new(Mode::Server), argv(&["bin", "run", "guest.wasm"]))
            .err()
            .expect("core cannot serve the `run` grammar");
        assert!(fatal(&error).contains("built without omnia's `cli` feature"));
    }

    #[cfg(unix)]
    #[test]
    fn direct_non_utf8() {
        use std::os::unix::ffi::OsStringExt as _;

        let options = MainOptions::new(Mode::Command).manifest(inline_source("app"));
        let bad = OsString::from_vec(vec![b'f', b'o', 0x80]);
        let error = plan(options, vec![OsString::from("bin"), bad])
            .err()
            .expect("non-UTF-8 argv must fail, not panic");
        assert!(fatal(&error).contains("not valid UTF-8"));
    }
}
