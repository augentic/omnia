//! Entry planning for the generated `main`: macro-compiled [`MainOptions`]
//! plus process argv and environment resolve into a deployment builder.
//!
//! [`plan`] is pure with respect to the process — argv and `OMNIA_CONFIG` are
//! parameters — so source precedence and argv policy are unit-testable
//! without spawning a binary.

use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Result, anyhow};
use clap::Parser as _;

use crate::cli::{Cli, Command};
use crate::dispatch::GuestResolver;
use crate::registry::GuestId;
use crate::runtime::Mode;
use crate::{DeploymentBuilder, Manifest};

/// How a runtime's compiled-in deployment manifest is supplied.
///
/// The `runtime!` macro emits [`Path`](Self::Path) for its `config:` key and
/// [`Inline`](Self::Inline) for its inline manifest keys (`guests`, `mounts`,
/// `link`, `routes`). On the standard CLI path it is the lowest-priority
/// source (behind `--config`/`OMNIA_CONFIG` and a positional wasm path);
/// under the macro's `program:` key it is the sole source.
#[derive(Clone, Debug)]
pub enum ManifestSource {
    /// A manifest path, loaded only when this source is selected.
    Path(PathBuf),
    /// A manifest value assembled at compile time.
    Inline(Manifest),
}

impl ManifestSource {
    /// Resolve into a manifest, loading the file for the path kind.
    fn into_manifest(self) -> Result<Manifest> {
        match self {
            Self::Path(path) => Manifest::from_config(path),
            Self::Inline(manifest) => Ok(manifest),
        }
    }
}

/// How the generated `main` is driven from the process boundary.
enum Invocation {
    /// Parse the standard `run [wasm] [--config] -- args…` grammar.
    OmniaCli,
    /// Raw argv passthrough: no host CLI grammar; every argument after the
    /// binary name belongs to the guest.
    DirectCommand {
        /// Deployment name for telemetry and command-mode `argv[0]`.
        program_name: String,
    },
}

/// Deployment options the `runtime!` macro compiles into the generated `main`.
#[doc(hidden)]
pub struct MainOptions {
    mode: Mode,
    manifest: Option<ManifestSource>,
    resolver: Option<Arc<dyn GuestResolver>>,
    invocation: Invocation,
    command_guest: Option<GuestId>,
}

impl MainOptions {
    /// Start options for a deployment driven in `mode`.
    #[must_use]
    pub const fn new(mode: Mode) -> Self {
        Self {
            mode,
            manifest: None,
            resolver: None,
            invocation: Invocation::OmniaCli,
            command_guest: None,
        }
    }

    /// Set the compiled-in manifest source (the macro's `config:` key or
    /// inline manifest keys).
    #[must_use]
    pub fn manifest(mut self, source: ManifestSource) -> Self {
        self.manifest = Some(source);
        self
    }

    /// Install a [`GuestResolver`] consulted on registry misses; a resolver
    /// also marks the deployment dynamic (its guest set may start empty).
    #[must_use]
    pub fn resolver<R: GuestResolver>(mut self, resolver: R) -> Self {
        self.resolver = Some(Arc::new(resolver));
        self
    }

    /// Disable the host CLI grammar (the macro's `program:` key): argv passes
    /// to the guest verbatim, the compiled-in manifest is the sole deployment
    /// source, and `program_name` becomes telemetry name and `argv[0]`.
    #[must_use]
    pub fn direct_command(mut self, program_name: impl Into<String>) -> Self {
        self.invocation = Invocation::DirectCommand {
            program_name: program_name.into(),
        };
        self
    }

    /// Route command mode to an explicit guest identity instead of the
    /// sole-static-exporter catch-all.
    #[must_use]
    pub fn command_guest(mut self, id: impl Into<GuestId>) -> Self {
        self.command_guest = Some(id.into());
        self
    }
}

/// Why entry planning stopped before a deployment could be built.
pub(super) enum PlanError {
    /// A clap-level outcome (usage error, `--help`, `--version`); the caller
    /// delegates to [`clap::Error::exit`] so stream and exit code match the
    /// standard CLI behavior.
    Usage(clap::Error),
    /// A startup failure reported on stderr.
    Fatal(anyhow::Error),
}

impl From<anyhow::Error> for PlanError {
    fn from(error: anyhow::Error) -> Self {
        Self::Fatal(error)
    }
}

/// The planner's outcome: every deployment decision, resolved.
pub(super) struct EntryPlan {
    mode: Mode,
    manifest: Option<Manifest>,
    args: Vec<String>,
    dynamic: bool,
    resolver: Option<Arc<dyn GuestResolver>>,
    command_guest: Option<GuestId>,
    program_name: Option<String>,
}

impl EntryPlan {
    /// Assemble the deployment builder this plan describes.
    pub(super) fn into_builder(self) -> DeploymentBuilder {
        let mut builder =
            DeploymentBuilder::new().manifest(self.manifest).args(self.args).mode(self.mode);
        if self.dynamic {
            builder = builder.dynamic();
        }
        if let Some(resolver) = self.resolver {
            builder = builder.resolver(resolver);
        }
        if let Some(id) = self.command_guest {
            builder = builder.command_guest(id);
        }
        if let Some(name) = self.program_name {
            builder = builder.program_name(name);
        }
        builder
    }
}

/// Resolve [`MainOptions`] plus process argv and `OMNIA_CONFIG` into an
/// [`EntryPlan`].
///
/// On the direct-command path the plan always carries either the compiled-in
/// manifest or the dynamic mark, so the builder never falls through to its
/// own `OMNIA_CONFIG` lookup — the environment is untouched by design.
pub(super) fn plan(
    options: MainOptions, argv: impl IntoIterator<Item = OsString>, omnia_config: Option<OsString>,
) -> Result<EntryPlan, PlanError> {
    let MainOptions {
        mode,
        manifest,
        resolver,
        invocation,
        command_guest,
    } = options;
    let dynamic = resolver.is_some();

    match invocation {
        Invocation::DirectCommand { program_name } => {
            let guest_args = argv
                .into_iter()
                .skip(1)
                .map(|arg| {
                    arg.into_string().map_err(|arg| {
                        anyhow!("guest argument `{}` is not valid UTF-8", arg.display())
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            let manifest = manifest.map(ManifestSource::into_manifest).transpose()?;
            if manifest.is_none() && !dynamic {
                return Err(PlanError::Fatal(anyhow!(
                    "a direct command deployment needs a compiled-in manifest or a resolver"
                )));
            }
            Ok(EntryPlan {
                mode,
                manifest,
                args: guest_args,
                dynamic,
                resolver,
                command_guest,
                program_name: Some(program_name),
            })
        }
        Invocation::OmniaCli => {
            let cli = Cli::try_parse_from(argv).map_err(PlanError::Usage)?;
            match cli.command {
                Command::Run {
                    wasm,
                    config,
                    mounts,
                    links,
                    args,
                } => {
                    let config = config.or_else(|| omnia_config.map(PathBuf::from));
                    let manifest = match (config, wasm) {
                        (Some(config), _) => Manifest::from_config(config)?,
                        (None, Some(wasm)) => Manifest::from_wasm(wasm),
                        (None, None) => match manifest {
                            Some(source) => source.into_manifest()?,
                            // A resolver-backed deployment may start empty.
                            None if dynamic => Manifest::new(),
                            None => {
                                return Err(PlanError::Fatal(anyhow!(
                                    "no guest specified: pass a <wasm> path, or --config \
                                     <omnia.toml> (or set OMNIA_CONFIG)"
                                )));
                            }
                        },
                    };
                    Ok(EntryPlan {
                        mode,
                        manifest: Some(manifest.mounts(mounts).links(links)),
                        args,
                        dynamic,
                        resolver,
                        command_guest,
                        program_name: None,
                    })
                }
                #[cfg(feature = "jit")]
                Command::Compile { .. } => Err(PlanError::Fatal(anyhow!(
                    "the generated `main` only supports `run`; supply a custom `main` for other \
                     subcommands"
                ))),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use futures::FutureExt as _;

    use super::*;
    use crate::deployment::{GuestArtifact, GuestEntry};
    use crate::host::FutureResult;

    struct NullResolver;

    impl GuestResolver for NullResolver {
        fn resolve(
            &self, _guest: GuestId, _expected_export: String,
        ) -> FutureResult<Option<GuestArtifact>> {
            async { Ok(None) }.boxed()
        }
    }

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

    fn temp_manifest(tag: &str, guest: &str) -> PathBuf {
        let path =
            std::env::temp_dir().join(format!("omnia_entry_{tag}_{}.toml", std::process::id()));
        std::fs::write(&path, format!("[[guest]]\nid = \"{guest}\"\nsource.path = \"./g.wasm\"\n"))
            .expect("temp manifest should write");
        path
    }

    fn fatal(error: PlanError) -> String {
        match error {
            PlanError::Fatal(error) => format!("{error:#}"),
            PlanError::Usage(error) => panic!("expected a fatal error, got usage: {error}"),
        }
    }

    #[test]
    fn config_beats_positional_wasm_and_compiled_source() {
        let path = temp_manifest("precedence", "from_config");
        let options = MainOptions::new(Mode::Server).manifest(inline_source("compiled"));
        let plan = plan(
            options,
            argv(&["bin", "run", "guest.wasm", "--config", path.to_str().unwrap()]),
            None,
        )
        .unwrap_or_else(|error| panic!("{}", fatal(error)));
        let _ = std::fs::remove_file(&path);
        assert_eq!(first_guest(&plan), "from_config");
    }

    #[test]
    fn omnia_config_env_beats_positional_wasm() {
        let path = temp_manifest("env", "from_env");
        let plan = plan(
            MainOptions::new(Mode::Server),
            argv(&["bin", "run", "guest.wasm"]),
            Some(path.clone().into_os_string()),
        )
        .unwrap_or_else(|error| panic!("{}", fatal(error)));
        let _ = std::fs::remove_file(&path);
        assert_eq!(first_guest(&plan), "from_env");
    }

    #[test]
    fn positional_wasm_beats_compiled_source() {
        let options = MainOptions::new(Mode::Server).manifest(inline_source("compiled"));
        let plan = plan(options, argv(&["bin", "run", "guest.wasm"]), None)
            .unwrap_or_else(|error| panic!("{}", fatal(error)));
        assert_eq!(first_guest(&plan), "guest");
    }

    #[test]
    fn compiled_source_is_the_fallback() {
        let options = MainOptions::new(Mode::Server).manifest(inline_source("compiled"));
        let plan = plan(options, argv(&["bin", "run"]), None)
            .unwrap_or_else(|error| panic!("{}", fatal(error)));
        assert_eq!(first_guest(&plan), "compiled");
    }

    #[test]
    fn no_source_and_no_resolver_fails() {
        let error = plan(MainOptions::new(Mode::Server), argv(&["bin", "run"]), None)
            .err()
            .expect("a sourceless static deployment must fail");
        assert!(fatal(error).contains("no guest specified"));
    }

    #[test]
    fn resolver_marks_dynamic_on_every_source() {
        // No source at all: the deployment starts empty rather than erroring.
        let options = MainOptions::new(Mode::Command).resolver(NullResolver).command_guest("app");
        let plan_empty = plan(options, argv(&["bin", "run"]), None)
            .unwrap_or_else(|error| panic!("{}", fatal(error)));
        assert!(plan_empty.dynamic);
        assert!(plan_empty.resolver.is_some());
        assert!(plan_empty.manifest.as_ref().is_some_and(|m| m.guests.is_empty()));
        assert_eq!(plan_empty.command_guest, Some(GuestId::from("app")));

        // A positional wasm source composes with the resolver unchanged.
        let options = MainOptions::new(Mode::Command).resolver(NullResolver);
        let plan_wasm = plan(options, argv(&["bin", "run", "guest.wasm"]), None)
            .unwrap_or_else(|error| panic!("{}", fatal(error)));
        assert!(plan_wasm.dynamic);
        assert!(plan_wasm.resolver.is_some());
        assert_eq!(first_guest(&plan_wasm), "guest");
    }

    #[test]
    fn direct_command_forwards_argv_verbatim() {
        // `--config` and `run` are guest arguments, not host CLI options.
        let options =
            MainOptions::new(Mode::Command).manifest(inline_source("app")).direct_command("myprog");
        let plan = plan(options, argv(&["bin", "--config", "foo.toml", "run", "greet"]), None)
            .unwrap_or_else(|error| panic!("{}", fatal(error)));
        assert_eq!(plan.args, ["--config", "foo.toml", "run", "greet"]);
        assert_eq!(plan.program_name.as_deref(), Some("myprog"));
        assert_eq!(first_guest(&plan), "app");
    }

    // Hard acceptance criterion: the direct path either carries a manifest or
    // the dynamic mark, so `DeploymentBuilder::build` can never fall through
    // to its own `OMNIA_CONFIG` lookup.
    #[test]
    fn direct_command_ignores_omnia_config() {
        let options = MainOptions::new(Mode::Command)
            .resolver(NullResolver)
            .command_guest("app")
            .direct_command("app");
        // The env names a nonexistent file; consulting it would fail loudly.
        let plan =
            plan(options, argv(&["bin", "greet"]), Some(OsString::from("/nonexistent/omnia.toml")))
                .unwrap_or_else(|error| panic!("{}", fatal(error)));
        assert!(plan.manifest.is_none(), "no compiled-in manifest, none loaded");
        assert!(plan.dynamic, "the dynamic mark keeps the builder off the env fallback");
        assert_eq!(plan.args, ["greet"]);
    }

    #[cfg(unix)]
    #[test]
    fn direct_command_non_utf8_argv_fails() {
        use std::os::unix::ffi::OsStringExt as _;

        let options =
            MainOptions::new(Mode::Command).manifest(inline_source("app")).direct_command("app");
        let bad = OsString::from_vec(vec![b'f', b'o', 0x80]);
        let error = plan(options, vec![OsString::from("bin"), bad], None)
            .err()
            .expect("non-UTF-8 argv must fail, not panic");
        assert!(fatal(error).contains("not valid UTF-8"));
    }

    #[test]
    fn direct_command_without_source_or_resolver_fails() {
        let options = MainOptions::new(Mode::Command).direct_command("app");
        let error = plan(options, argv(&["bin"]), None)
            .err()
            .expect("a direct command with nothing to run must fail");
        assert!(fatal(error).contains("compiled-in manifest or a resolver"));
    }

    #[test]
    fn compiled_path_load_failure_surfaces() {
        let options = MainOptions::new(Mode::Server)
            .manifest(ManifestSource::Path(PathBuf::from("/nonexistent/omnia.toml")));
        let error = plan(options, argv(&["bin", "run"]), None)
            .err()
            .expect("a missing compiled-in manifest path must fail");
        assert!(fatal(error).contains("reading manifest"));
    }

    #[test]
    fn usage_error_is_delegated_to_clap() {
        let error = plan(MainOptions::new(Mode::Server), argv(&["bin", "bogus"]), None)
            .err()
            .expect("an unknown subcommand is a usage error");
        assert!(matches!(error, PlanError::Usage(_)));
    }
}
