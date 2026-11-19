//! The clap-backed half of the command façade: subcommand routes, argv
//! parsing, and shell completions.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fmt;
use std::future::Future;
use std::marker::PhantomData;
use std::path::Path;
use std::pin::Pin;
use std::sync::Arc;

pub use clap_complete::Shell;
use serde::Serialize;
use tracing::Instrument as _;

use super::{Command, Format, HintFn, Render, Response, USAGE_EXIT};
use crate::{Handler, Handlers, Metadata, Router};

impl Response {
    /// A usage error rendered by clap, exiting [`USAGE_EXIT`].
    #[must_use]
    pub fn usage(error: &clap::Error) -> Self {
        Self::failure(error.render().to_string(), USAGE_EXIT)
    }
}

/// The outcome of parsing argv: the grammar, or one of clap's own complete
/// responses.
#[derive(Debug)]
pub enum Parsed<A> {
    /// The parsed grammar.
    App(A),

    /// Help or version text destined for stdout with exit 0.
    Display(String),

    /// A usage error destined for stderr; see [`Response::usage`].
    Usage(clap::Error),
}

/// The runtime's verbosity flags, declared in a command's grammar by
/// flattening.
///
/// The runtime reads `-v`/`--verbose` and `-q`/`--quiet` out of argv before
/// the command runs and sets its `RUST_LOG` from them — each `-v` one level
/// more detail, each `-q` one less, from the command's default of `info` — so
/// the command never acts on these values. Declaring them keeps the parser
/// accepting what the runtime already honoured, lists them in `--help` and
/// completions, and refuses `-v` beside `-q` as a usage error (the runtime
/// applies no level for that pair however argv splits it). A
/// [`Router`] declares them itself.
///
/// ```rust,ignore
/// #[derive(clap::Parser)]
/// struct App {
///     #[command(flatten)]
///     verbosity: omnia_sdk::command::Verbosity,
///
///     #[command(subcommand)]
///     verb: Verb,
/// }
/// ```
#[derive(clap::Args, Clone, Copy, Debug, Default, Eq, PartialEq)]
// A flattened struct's doc comment would otherwise become the parent command's `about` and
// `long_about`; clearing both here leaves the grammar's own text in place.
#[command(about = None, long_about = None)]
pub struct Verbosity {
    /// Show more detail, once per level.
    #[arg(short, long, action = clap::ArgAction::Count, global = true)]
    pub verbose: u8,

    /// Show less detail, once per level.
    #[arg(short, long, action = clap::ArgAction::Count, global = true, conflicts_with = "verbose")]
    pub quiet: u8,
}

#[derive(Clone, Default)]
pub struct Commands {
    routes: BTreeMap<String, ErasedCommand>,
}

impl Commands {
    pub fn route<F, I>(self, name: impl Into<String>, handler: F) -> Self
    where
        F: Handler<I>,
        F::Output: Serialize + Render,
        I: clap::Args + Send + 'static,
    {
        let name = name.into();
        assert!(!name.is_empty(), "command name cannot be empty");
        let command = I::augment_args(clap::Command::new(name));
        self.route_with(command, handler, |matches| {
            I::from_arg_matches(matches).map_err(|error| crate::bad_request!(error))
        })
    }

    pub fn route_with<F, I, D>(self, command: clap::Command, handler: F, decode: D) -> Self
    where
        F: Handler<I>,
        F::Output: Serialize + Render,
        I: Send + 'static,
        D: Fn(&clap::ArgMatches) -> crate::Result<I> + Send + Sync + 'static,
    {
        let name = command.get_name().to_owned();
        let route = Route::<F, I, D> {
            handler,
            decode,
            marker: PhantomData,
        };
        self.insert(
            name,
            ErasedCommand {
                command,
                route: Arc::new(route),
            },
        )
    }

    pub fn merge(self, other: Self) -> Self {
        other.routes.into_iter().fold(self, |routes, (name, entry)| routes.insert(name, entry))
    }

    pub fn commands(&self) -> impl Iterator<Item = &str> {
        self.routes.keys().map(String::as_str)
    }

    fn insert(mut self, name: String, entry: ErasedCommand) -> Self {
        assert!(!name.is_empty(), "command name cannot be empty");
        assert!(!self.routes.contains_key(&name), "duplicate command `{name}`");
        self.routes.insert(name, entry);
        self
    }
}

impl fmt::Debug for Commands {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_set().entries(self.routes.keys()).finish()
    }
}

#[derive(Clone)]
struct ErasedCommand {
    command: clap::Command,
    route: Arc<dyn ErasedRoute>,
}

type DispatchFuture<'a> = Pin<Box<dyn Future<Output = Response> + Send + 'a>>;

trait ErasedRoute: Send + Sync {
    fn dispatch<'a>(
        &'a self, matches: &'a clap::ArgMatches, command: &'a Command<'a>,
    ) -> DispatchFuture<'a>;
}

struct Route<F, I, D> {
    handler: F,
    decode: D,
    marker: PhantomData<fn() -> I>,
}

impl<F, I, D> ErasedRoute for Route<F, I, D>
where
    F: Handler<I>,
    F::Output: Serialize + Render,
    I: Send + 'static,
    D: Fn(&clap::ArgMatches) -> crate::Result<I> + Send + Sync + 'static,
{
    fn dispatch<'a>(
        &'a self, matches: &'a clap::ArgMatches, command: &'a Command<'a>,
    ) -> DispatchFuture<'a> {
        Box::pin(async move {
            match (self.decode)(matches) {
                Ok(input) => command.call(self.handler.clone(), input).await,
                Err(error) => command.refuse(error),
            }
        })
    }
}

// The subcommands a closed router runs, with the program identity
// clap shows around them.
#[derive(Clone)]
pub struct Program {
    pub commands: Commands,
    pub name: Option<String>,
    pub version: Option<String>,
    pub about: Option<String>,
    pub env_prefix: Option<String>,
    pub hint: Option<HintFn<'static>>,
}

impl Program {
    pub const fn new(commands: Commands) -> Self {
        Self {
            commands,
            name: None,
            version: None,
            about: None,
            env_prefix: None,
            hint: None,
        }
    }

    // Cargo stamps an empty description where the manifest sets none.
    pub fn package(&mut self, version: &str, description: &str) {
        self.version.get_or_insert_with(|| version.to_owned());
        if !description.is_empty() {
            self.about.get_or_insert_with(|| description.to_owned());
        }
    }

    pub fn completions(&self, shell: Shell) -> Response {
        let name = self.name.as_deref().unwrap_or("app");
        let mut command = self.build_command(name);
        let mut out = Vec::new();
        clap_complete::generate(shell, &mut command, name, &mut out);
        Response::success(out)
    }

    pub async fn run(&self, argv: impl IntoIterator<Item: Into<OsString>>) -> Response {
        let argv: Vec<OsString> = argv.into_iter().map(Into::into).collect();
        let bin = self.bin_name(&argv);
        let prefix =
            self.env_prefix.clone().unwrap_or_else(|| bin.to_ascii_uppercase().replace('-', "_"));
        let metadata = Metadata::from_env(&prefix);
        self.dispatch(argv, &metadata).await
    }

    pub async fn run_with(
        &self, argv: impl IntoIterator<Item: Into<OsString>>, metadata: &Metadata,
    ) -> Response {
        let argv: Vec<OsString> = argv.into_iter().map(Into::into).collect();
        self.dispatch(argv, metadata).await
    }

    fn bin_name<'a>(&'a self, argv: &'a [OsString]) -> &'a str {
        self.name
            .as_deref()
            .or_else(|| {
                argv.first()
                    .and_then(|arg| Path::new(arg).file_name())
                    .and_then(|name| name.to_str())
                    .filter(|name| !name.is_empty())
            })
            .unwrap_or("app")
    }

    // `Verbosity` clears the about it is flattened into, so the
    // program's own text goes on after the globals.
    fn build_command(&self, bin_name: &str) -> clap::Command {
        let mut command = <Globals as clap::Args>::augment_args(
            clap::Command::new(bin_name.to_owned())
                .bin_name(bin_name.to_owned())
                .subcommand_required(true),
        );
        if let Some(version) = &self.version {
            command = command.version(version.clone());
        }
        if let Some(about) = &self.about {
            command = command.about(about.clone());
        }
        for entry in self.commands.routes.values() {
            command = command.subcommand(entry.command.clone());
        }
        command
    }

    async fn dispatch(&self, argv: Vec<OsString>, metadata: &Metadata) -> Response {
        let bin = self.bin_name(&argv).to_owned();
        let command = self.build_command(&bin);
        let matches = match command.try_get_matches_from(argv) {
            Ok(matches) => matches,
            Err(error) if error.use_stderr() => return Response::usage(&error),
            Err(display) => return Response::success(display.render().to_string()),
        };
        let globals = <Globals as clap::FromArgMatches>::from_arg_matches(&matches)
            .expect("globals augmented on command");
        let (name, sub_matches) = matches.subcommand().expect("subcommand_required is enabled");
        let entry = self.commands.routes.get(name).expect("matched subcommand is registered");
        let mut projector = Command::new(metadata, globals.format);
        if let Some(hint) = &self.hint {
            projector.hint = Some(Arc::clone(hint));
        }
        let span = tracing::info_span!(
            "handler",
            request_id = metadata.request_id.as_deref(),
            correlation_id = metadata.correlation_id.as_deref(),
            command = name,
        );
        entry.route.dispatch(sub_matches, &projector).instrument(span).await
    }
}

impl fmt::Debug for Program {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Program")
            .field("commands", &self.commands)
            .field("name", &self.name)
            .field("version", &self.version)
            .field("about", &self.about)
            .field("env_prefix", &self.env_prefix)
            .field("hint", &self.hint.is_some())
            .finish()
    }
}

#[derive(clap::Args)]
struct Globals {
    /// Output format for every verb.
    #[arg(long, default_value = "text", global = true)]
    format: Format,

    #[command(flatten)]
    verbosity: Verbosity,
}

/// Parse argv into the grammar `A`, classifying clap's own outcomes.
#[must_use]
pub fn parse<A: clap::Parser>(argv: impl IntoIterator<Item: Into<OsString> + Clone>) -> Parsed<A> {
    match A::try_parse_from(argv) {
        Ok(app) => Parsed::App(app),
        Err(error) if error.use_stderr() => Parsed::Usage(error),
        Err(display) => Parsed::Display(display.render().to_string()),
    }
}

/// A shell-completion script for the grammar `A` under the binary `name`.
#[must_use]
pub fn completions<A: clap::CommandFactory>(shell: Shell, name: &str) -> Response {
    let mut out = Vec::new();
    clap_complete::generate(shell, &mut A::command(), name, &mut out);
    Response::success(out)
}

impl Handlers {
    /// Register one handler for one subcommand, decoding its arguments
    /// through [`clap::Args`].
    ///
    /// The input's doc comment is the subcommand's `--help` text; the
    /// output's [`Serialize`] and [`Render`] forms answer
    /// `--format json` and `--format text`.
    ///
    /// # Panics
    ///
    /// Panics when `name` is empty or already registered.
    pub fn command<F, I>(mut self, name: impl Into<String>, handler: F) -> Self
    where
        F: Handler<I>,
        F::Output: Serialize + Render,
        I: clap::Args + Send + 'static,
    {
        self.commands = self.commands.route(name, handler);
        self
    }

    /// Register one handler for one [`clap::Command`] subcommand with a
    /// decoder over its [`clap::ArgMatches`].
    ///
    /// A decoder's refusal fails the run with its own
    /// [`Error`](crate::Error), as a handler's error does.
    ///
    /// # Panics
    ///
    /// Panics when the subcommand's name is empty or already registered.
    pub fn command_with<F, I, D>(mut self, command: clap::Command, handler: F, decode: D) -> Self
    where
        F: Handler<I>,
        F::Output: Serialize + Render,
        I: Send + 'static,
        D: Fn(&clap::ArgMatches) -> crate::Result<I> + Send + Sync + 'static,
    {
        self.commands = self.commands.route_with(command, handler, decode);
        self
    }
}

impl Router {
    /// Set the program name shown in usage and help text.
    ///
    /// Defaults to `argv[0]` (or `"app"` when `argv` is empty).
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.program.name = Some(name.into());
        self
    }

    /// Set the version answered for `--version`.
    ///
    /// Defaults to the crate's `CARGO_PKG_VERSION` under
    /// [`export!`](super::export!).
    pub fn version(mut self, version: impl Into<String>) -> Self {
        self.program.version = Some(version.into());
        self
    }

    /// Set the description shown at the top of `--help`.
    ///
    /// Defaults to the crate's `CARGO_PKG_DESCRIPTION` under
    /// [`export!`](super::export!).
    pub fn about(mut self, about: impl Into<String>) -> Self {
        self.program.about = Some(about.into());
        self
    }

    /// Set the environment-variable prefix [`run`](Self::run) passes to
    /// [`Metadata::from_env`].
    ///
    /// Defaults to the program name in uppercase with `-` replaced by `_`.
    pub fn env_prefix(mut self, prefix: impl Into<String>) -> Self {
        self.program.env_prefix = Some(prefix.into());
        self
    }

    /// Attach a remedy-hint fn consulted for every command failure.
    pub fn hints(
        mut self, hint: impl Fn(&crate::Error) -> Option<Cow<'static, str>> + Send + Sync + 'static,
    ) -> Self {
        self.program.hint = Some(Arc::new(hint));
        self
    }

    /// The subcommands registered on this router, in order.
    pub fn commands(&self) -> impl Iterator<Item = &str> {
        self.program.commands.commands()
    }

    /// A shell-completion script for this router's command grammar.
    #[must_use]
    pub fn completions(&self, shell: Shell) -> Response {
        self.program.completions(shell)
    }

    /// Run argv: parse it, read [`Metadata::from_env`], and dispatch the
    /// matched subcommand.
    pub async fn run(&self, argv: impl IntoIterator<Item: Into<OsString>>) -> Response {
        self.program.run(argv).await
    }

    /// Run argv with explicit `metadata`.
    pub async fn run_with(
        &self, argv: impl IntoIterator<Item: Into<OsString>>, metadata: &Metadata,
    ) -> Response {
        self.program.run_with(argv, metadata).await
    }

    // The defaults `export!` stamps from the guest crate's manifest.
    // A version or description the router already set stands.
    #[doc(hidden)]
    pub fn package(mut self, version: &str, description: &str) -> Self {
        self.program.package(version, description);
        self
    }
}
