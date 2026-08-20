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
#[cfg(feature = "cli")]
use clap::Parser as _;

#[cfg(feature = "cli")]
use crate::cli::{Cli, Command};
use crate::dispatch::{GuestResolver, HttpPaths};
use crate::registry::GuestId;
use crate::runtime::Mode;
use crate::telemetry::LogMode;
use crate::{DeploymentBuilder, Manifest};

/// How a runtime's compiled-in deployment manifest is supplied.
///
/// The `runtime!` macro emits [`Path`](Self::Path) for its `config:` key and
/// [`Inline`](Self::Inline) for its inline manifest keys (`guests`, `mounts`,
/// `link`, `routes`). On the standard CLI path it is the lowest-priority
/// source (behind `--config`/`OMNIA_CONFIG` and a positional wasm path); on
/// the direct-command path it is the sole source.
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

/// Deployment options the `runtime!` macro compiles into the generated `main`.
#[doc(hidden)]
pub struct MainOptions {
    mode: Mode,
    manifest: Option<ManifestSource>,
    resolver: Option<Arc<dyn GuestResolver>>,
    http_paths: Option<HttpPaths>,
    http_listener: Option<std::net::TcpListener>,
}

impl MainOptions {
    /// Start options for a deployment driven in `mode`.
    #[must_use]
    pub const fn new(mode: Mode) -> Self {
        Self {
            mode,
            manifest: None,
            resolver: None,
            http_paths: None,
            http_listener: None,
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

    /// Install the [`HttpPaths`] hook: the deployment's path→identity
    /// mapping for request paths no static route matches. Installing one
    /// makes HTTP routing table-driven only — a sole exporter never becomes
    /// a catch-all, a path the hook declines (or an identity nothing
    /// supplies) is an ordinary 404, and a resolution fault or a guest
    /// without the handler export is a 500.
    #[must_use]
    pub fn http_paths<F>(mut self, hook: F) -> Self
    where
        F: Fn(&str) -> Option<GuestId> + Send + Sync + 'static,
    {
        self.http_paths = Some(Arc::new(hook));
        self
    }

    /// Supply a pre-bound TCP listener for the HTTP trigger. The trigger
    /// server adopts it at boot instead of binding `HTTP_ADDR` itself, and
    /// every guest store sees `HTTP_ADDR` set to its local address.
    #[must_use]
    pub fn http_listener(mut self, listener: std::net::TcpListener) -> Self {
        self.http_listener = Some(listener);
        self
    }
}

/// Why entry planning stopped before a deployment could be built.
pub(super) enum PlanError {
    /// A clap-level outcome (usage error, `--help`, `--version`); the caller
    /// delegates to [`clap::Error::exit`] so stream and exit code match the
    /// standard CLI behavior.
    #[cfg(feature = "cli")]
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
    http_paths: Option<HttpPaths>,
    http_listener: Option<std::net::TcpListener>,
    log_mode: Option<LogMode>,
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
        if let Some(hook) = self.http_paths {
            builder = builder.http_paths_shared(hook);
        }
        if let Some(listener) = self.http_listener {
            builder = builder.http_listener(listener);
        }
        if let Some(mode) = self.log_mode {
            builder = builder.log_mode(mode);
        }
        builder
    }
}

/// Peel the reserved host log flags (`--debug` / `--quiet`) out of
/// direct-command argv, returning the guest arguments and the resolved
/// [`LogMode`] (the flagless default is [`LogMode::Progress`]).
///
/// The flags are host-reserved anywhere in argv — a direct-command guest
/// never sees them — and mutually exclusive; repeating one is idempotent.
fn peel_log_flags(args: Vec<String>) -> Result<(Vec<String>, LogMode)> {
    let mut mode = None;
    let mut guest_args = Vec::with_capacity(args.len());
    for arg in args {
        let flag = match arg.as_str() {
            "--debug" => LogMode::Debug,
            "--quiet" => LogMode::Quiet,
            _ => {
                guest_args.push(arg);
                continue;
            }
        };
        if mode.is_some_and(|current| current != flag) {
            return Err(anyhow!("`--debug` and `--quiet` are mutually exclusive"));
        }
        mode = Some(flag);
    }
    Ok((guest_args, mode.unwrap_or(LogMode::Progress)))
}

/// Resolve [`MainOptions`] plus process argv and `OMNIA_CONFIG` into an
/// [`EntryPlan`].
///
/// Command mode with a compiled-in manifest is a *direct command*: no host
/// CLI grammar, argv belongs to the guest. The direct plan always carries
/// the compiled-in manifest, so the builder never falls through to its own
/// `OMNIA_CONFIG` lookup — the environment is untouched by design. Every
/// other shape parses the standard `run [wasm] [--config] -- args…` grammar.
// Without `cli`, `omnia_config` is only acknowledged, never consumed.
#[cfg_attr(not(feature = "cli"), allow(clippy::needless_pass_by_value))]
pub(super) fn plan(
    options: MainOptions, argv: impl IntoIterator<Item = OsString>, omnia_config: Option<OsString>,
) -> Result<EntryPlan, PlanError> {
    let MainOptions {
        mode,
        manifest,
        resolver,
        http_paths,
        http_listener,
    } = options;
    let dynamic = resolver.is_some();

    if mode == Mode::Command && manifest.is_some() {
        let raw_args = argv
            .into_iter()
            .skip(1)
            .map(|arg| {
                arg.into_string()
                    .map_err(|arg| anyhow!("guest argument `{}` is not valid UTF-8", arg.display()))
            })
            .collect::<Result<Vec<_>>>()?;
        let (guest_args, log_mode) = peel_log_flags(raw_args)?;
        let manifest = manifest.map(ManifestSource::into_manifest).transpose()?;
        return Ok(EntryPlan {
            mode,
            manifest,
            args: guest_args,
            dynamic,
            resolver,
            http_paths,
            http_listener,
            log_mode: Some(log_mode),
        });
    }

    // Without the `cli` feature the standard grammar cannot be parsed; only
    // direct-command deployments are entry points.
    #[cfg(not(feature = "cli"))]
    {
        let _ = omnia_config;
        Err(PlanError::Fatal(anyhow!(
            "this runtime was built without omnia's `cli` feature; compile the deployment in \
             (command mode with a manifest) or enable the feature"
        )))
    }
    #[cfg(feature = "cli")]
    {
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
                                "no guest specified: pass a <wasm> path, or --config <omnia.toml> \
                                 (or set OMNIA_CONFIG)"
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
                    http_paths,
                    http_listener,
                    log_mode: None,
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

// Unit tests by design: `plan` is factored pure (argv and `OMNIA_CONFIG` are
// parameters) precisely so source precedence and argv policy are testable
// without spawning a binary; everything downstream of `into_builder` —
// deployment build, command routing, exit mapping — is the `cli` seam
// module's job.
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

    #[cfg(feature = "cli")]
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
            #[cfg(feature = "cli")]
            PlanError::Usage(error) => panic!("expected a fatal error, got usage: {error}"),
        }
    }

    #[cfg(feature = "cli")]
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

    #[cfg(feature = "cli")]
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

    #[cfg(feature = "cli")]
    #[test]
    fn positional_wasm_beats_compiled_source() {
        let options = MainOptions::new(Mode::Server).manifest(inline_source("compiled"));
        let plan = plan(options, argv(&["bin", "run", "guest.wasm"]), None)
            .unwrap_or_else(|error| panic!("{}", fatal(error)));
        assert_eq!(first_guest(&plan), "guest");
    }

    #[cfg(feature = "cli")]
    #[test]
    fn compiled_source_is_the_fallback() {
        let options = MainOptions::new(Mode::Server).manifest(inline_source("compiled"));
        let plan = plan(options, argv(&["bin", "run"]), None)
            .unwrap_or_else(|error| panic!("{}", fatal(error)));
        assert_eq!(first_guest(&plan), "compiled");
    }

    #[cfg(feature = "cli")]
    #[test]
    fn no_source_and_no_resolver_fails() {
        let error = plan(MainOptions::new(Mode::Server), argv(&["bin", "run"]), None)
            .err()
            .expect("a sourceless static deployment must fail");
        assert!(fatal(error).contains("no guest specified"));
    }

    #[cfg(feature = "cli")]
    #[test]
    fn resolver_marks_dynamic_on_every_source() {
        // No source at all: the deployment starts empty rather than erroring.
        let options = MainOptions::new(Mode::Server).resolver(NullResolver);
        let plan_empty = plan(options, argv(&["bin", "run"]), None)
            .unwrap_or_else(|error| panic!("{}", fatal(error)));
        assert!(plan_empty.dynamic);
        assert!(plan_empty.resolver.is_some());
        assert!(plan_empty.manifest.as_ref().is_some_and(|m| m.guests.is_empty()));

        // A positional wasm source composes with the resolver unchanged; a
        // resolver without a compiled-in manifest keeps the `run` grammar
        // even in command mode.
        let options = MainOptions::new(Mode::Command).resolver(NullResolver);
        let plan_wasm = plan(options, argv(&["bin", "run", "guest.wasm"]), None)
            .unwrap_or_else(|error| panic!("{}", fatal(error)));
        assert!(plan_wasm.dynamic);
        assert!(plan_wasm.resolver.is_some());
        assert_eq!(first_guest(&plan_wasm), "guest");
    }

    #[cfg(feature = "cli")]
    #[test]
    fn command_mode_without_deployment_keeps_run_grammar() {
        let plan = plan(MainOptions::new(Mode::Command), argv(&["bin", "run", "guest.wasm"]), None)
            .unwrap_or_else(|error| panic!("{}", fatal(error)));
        assert_eq!(first_guest(&plan), "guest");
        assert_eq!(plan.log_mode, None, "the standard CLI path stays env-driven");
    }

    #[test]
    fn direct_command_forwards_argv_verbatim() {
        // `--config` and `run` are guest arguments, not host CLI options.
        let options = MainOptions::new(Mode::Command).manifest(inline_source("app"));
        let plan = plan(options, argv(&["bin", "--config", "foo.toml", "run", "greet"]), None)
            .unwrap_or_else(|error| panic!("{}", fatal(error)));
        assert_eq!(plan.args, ["--config", "foo.toml", "run", "greet"]);
        assert_eq!(plan.log_mode, Some(LogMode::Progress), "flagless default is progress");
        assert_eq!(first_guest(&plan), "app");
    }

    #[test]
    fn direct_command_peels_log_flags() {
        // The reserved flags are host-only wherever they sit in argv; the
        // guest arguments are otherwise untouched.
        let cases: &[(&[&str], LogMode, &[&str])] = &[
            (&["bin", "--debug", "plan", "author"], LogMode::Debug, &["plan", "author"]),
            (&["bin", "plan", "author", "--debug"], LogMode::Debug, &["plan", "author"]),
            (&["bin", "plan", "--quiet", "status"], LogMode::Quiet, &["plan", "status"]),
            (&["bin", "--debug", "run", "--debug"], LogMode::Debug, &["run"]),
            (&["bin"], LogMode::Progress, &[]),
        ];
        for (args, mode, guest_args) in cases {
            let options = MainOptions::new(Mode::Command).manifest(inline_source("app"));
            let plan =
                plan(options, argv(args), None).unwrap_or_else(|error| panic!("{}", fatal(error)));
            assert_eq!(plan.log_mode, Some(*mode), "argv: {args:?}");
            assert_eq!(plan.args, *guest_args, "argv: {args:?}");
        }
    }

    #[test]
    fn direct_command_debug_and_quiet_conflict() {
        let options = MainOptions::new(Mode::Command).manifest(inline_source("app"));
        let error = plan(options, argv(&["bin", "--debug", "greet", "--quiet"]), None)
            .err()
            .expect("conflicting log flags must fail");
        assert!(fatal(error).contains("mutually exclusive"));
    }

    #[cfg(feature = "cli")]
    #[test]
    fn omnia_cli_carries_no_log_mode() {
        let options = MainOptions::new(Mode::Server).manifest(inline_source("compiled"));
        let plan = plan(options, argv(&["bin", "run"]), None)
            .unwrap_or_else(|error| panic!("{}", fatal(error)));
        assert_eq!(plan.log_mode, None, "the standard CLI path stays env-driven");
    }

    // Hard acceptance criterion: the direct path always carries the
    // compiled-in manifest, so `DeploymentBuilder::build` can never fall
    // through to its own `OMNIA_CONFIG` lookup.
    #[test]
    fn direct_command_ignores_omnia_config() {
        let options = MainOptions::new(Mode::Command).manifest(inline_source("app"));
        // The env names a nonexistent file; consulting it would fail loudly.
        let plan =
            plan(options, argv(&["bin", "greet"]), Some(OsString::from("/nonexistent/omnia.toml")))
                .unwrap_or_else(|error| panic!("{}", fatal(error)));
        assert_eq!(first_guest(&plan), "app", "the compiled-in manifest is the sole source");
        assert_eq!(plan.args, ["greet"]);
    }

    #[cfg(unix)]
    #[test]
    fn direct_command_non_utf8_argv_fails() {
        use std::os::unix::ffi::OsStringExt as _;

        let options = MainOptions::new(Mode::Command).manifest(inline_source("app"));
        let bad = OsString::from_vec(vec![b'f', b'o', 0x80]);
        let error = plan(options, vec![OsString::from("bin"), bad], None)
            .err()
            .expect("non-UTF-8 argv must fail, not panic");
        assert!(fatal(error).contains("not valid UTF-8"));
    }

    #[cfg(feature = "cli")]
    #[test]
    fn compiled_path_load_failure_surfaces() {
        let options = MainOptions::new(Mode::Server)
            .manifest(ManifestSource::Path(PathBuf::from("/nonexistent/omnia.toml")));
        let error = plan(options, argv(&["bin", "run"]), None)
            .err()
            .expect("a missing compiled-in manifest path must fail");
        assert!(fatal(error).contains("reading manifest"));
    }

    #[cfg(feature = "cli")]
    #[test]
    fn usage_error_is_delegated_to_clap() {
        let error = plan(MainOptions::new(Mode::Server), argv(&["bin", "bogus"]), None)
            .err()
            .expect("an unknown subcommand is a usage error");
        assert!(matches!(error, PlanError::Usage(_)));
    }
}
