//! The command façade: argv in, buffered channels and an exit status out.
//!
//! A module registers its subcommands on a [`Commands`] of its own and the
//! guest merges each (`Commands::from([..])` or [`Commands::merge`]) into its
//! one [`Router`] over a [`Client`], closing the whole with
//! [`Commands::with_state`] as [`Handlers`](crate::api::Handlers) does. Each
//! subcommand decodes its input through [`clap::Args`] (or a custom decoder
//! registered with [`Commands::route_with`]), runs the [`Handler`] through
//! [`Client::call`], and encodes a success body onto stdout in the selected [`Format`] or a
//! [`Failure`] envelope onto stderr at [`Error::exit_code`](crate::Error::exit_code);
//! a clap usage error exits [`USAGE_EXIT`]. [`export!`] binds the router fn
//! (or a zero-argument `async fn` entry) as the `wasi:cli/run` export and
//! [`IntoExit`] writes the channels at that boundary.
//!
//! ```rust,ignore
//! use std::fmt;
//!
//! use clap::Args;
//! use omnia_sdk::api::command::{self, Commands, Router};
//! use omnia_sdk::api::{Client, Context};
//!
//! #[derive(Args)]
//! struct Greet {
//!     name: String,
//! }
//!
//! async fn greet<P>(input: Greet, _context: Context<P>) -> omnia_sdk::Result<String> {
//!     Ok(format!("hello, {}", input.name))
//! }
//!
//! fn render_greeting(greeting: &String, out: &mut dyn fmt::Write) -> fmt::Result {
//!     writeln!(out, "{greeting}")
//! }
//!
//! pub fn command_router<P: Send + Sync + 'static>(provider: P) -> Router<P> {
//!     Commands::new()
//!         .route("greet", greet, render_greeting)
//!         .with_state(Client::new(provider))
//! }
//!
//! command::export!(command_router);
//! ```

use std::borrow::Cow;
use std::fmt;
use std::io::{ErrorKind, Write};

use serde::{Serialize, Serializer};

use crate::api::{Client, ErrorBody, Format, Handler, Metadata};

#[doc(hidden)]
#[macro_export]
macro_rules! __export_command {
    ($entry:path $(,)?) => {
        $crate::api::command::export!($entry, $crate::WasiProvider);
    };
    ($entry:path, $provider:expr $(,)?) => {
        #[cfg(all(target_arch = "wasm32", target_os = "wasi"))]
        const _: () = {
            struct CliGuest;

            $crate::wasip3::cli::command::export!(CliGuest);

            impl $crate::wasip3::exports::cli::run::Guest for CliGuest {
                async fn run() -> ::core::result::Result<(), ()> {
                    $crate::api::command::Export::run($entry, $provider).await;
                    Ok(())
                }
            }
        };
    };
}

/// Bind a command router fn (or a zero-argument `async fn` entry) as the
/// guest's `wasi:cli/run` export.
///
/// Calls a router fn with [`WasiProvider`](crate::WasiProvider) (or with the
/// provider expression passed as a second argument) and dispatches the WASI
/// command-line arguments through the resulting [`Router`], or awaits a
/// zero-argument `async fn` returning [`IntoExit`], inside the guest
/// telemetry scope. Expands to nothing on native targets, so the same
/// crate's tests build the router natively.
///
/// ```rust,ignore
/// use omnia_sdk::api::command;
///
/// command::export!(command_router);
/// ```
#[doc(inline)]
pub use crate::__export_command as export;

/// The exit status of a command-line usage error (`EX_USAGE`).
pub const USAGE_EXIT: u8 = 64;

/// Buffered command output and exit status.
///
/// Both channels are whole buffers written at the exit boundary; a command
/// that must stream writes to the process channels itself.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Response {
    /// Bytes written to standard output.
    pub stdout: Vec<u8>,

    /// Bytes written to standard error.
    pub stderr: Vec<u8>,

    /// The process exit status.
    pub exit: u8,
}

impl Response {
    /// A successful response carrying stdout.
    #[must_use]
    pub fn success(stdout: impl Into<Vec<u8>>) -> Self {
        Self {
            stdout: stdout.into(),
            stderr: Vec::new(),
            exit: 0,
        }
    }

    /// A failed response carrying stderr and its exit status.
    #[must_use]
    pub fn failure(stderr: impl Into<Vec<u8>>, exit: u8) -> Self {
        Self {
            stdout: Vec::new(),
            stderr: stderr.into(),
            exit,
        }
    }
}

/// The exit status an [`export!`] entry yields: `()` always succeeds,
/// `Result<(), u8>` passes its code through, and [`Response`] writes its
/// channels first.
///
/// A scenario that asserts internally and traps on failure returns `()`; a
/// CLI that writes its own channels reports `Result<(), u8>`.
pub trait IntoExit {
    /// The status handed to the WASI CLI boundary.
    ///
    /// # Errors
    ///
    /// Returns the non-zero exit code the guest reports.
    fn into_exit(self) -> Result<(), u8>;
}

impl IntoExit for Response {
    fn into_exit(self) -> Result<(), u8> {
        write_channel(&mut std::io::stdout(), &self.stdout)?;
        write_channel(&mut std::io::stderr(), &self.stderr)?;
        if self.exit == 0 { Ok(()) } else { Err(self.exit) }
    }
}

impl IntoExit for () {
    fn into_exit(self) -> Result<(), u8> {
        Ok(())
    }
}

impl IntoExit for Result<(), u8> {
    fn into_exit(self) -> Result<(), u8> {
        self
    }
}

/// The failure envelope a command reports: the error, its exit status, and
/// an optional remedy hint.
#[derive(Clone, Debug)]
pub struct Failure {
    error: crate::Error,
    hint: Option<Cow<'static, str>>,
}

impl Failure {
    /// The underlying error.
    #[must_use]
    pub const fn error(&self) -> &crate::Error {
        &self.error
    }

    /// The transport-neutral wire body shared with HTTP.
    #[must_use]
    pub fn body(&self) -> ErrorBody {
        ErrorBody::from(&self.error)
    }

    /// The exit status from the error's exit map.
    #[must_use]
    pub const fn exit_code(&self) -> u8 {
        self.error.exit_code()
    }

    /// The remedy hint, when one was attached.
    #[must_use]
    pub fn hint(&self) -> Option<&str> {
        self.hint.as_deref()
    }

    /// Attach a remedy hint.
    #[must_use]
    pub fn with_hint(mut self, hint: impl Into<Cow<'static, str>>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    /// Render the envelope as `error[<code>]: <message>` and an optional
    /// `hint:` line; the text-mode counterpart of its JSON form.
    ///
    /// # Errors
    ///
    /// Propagates the sink's formatting failure.
    pub fn text(&self, out: &mut dyn fmt::Write) -> fmt::Result {
        let ErrorBody { error, message } = self.body();
        writeln!(out, "error[{error}]: {message}")?;
        if let Some(hint) = self.hint() {
            writeln!(out, "hint: {hint}")?;
        }
        Ok(())
    }
}

impl From<crate::Error> for Failure {
    fn from(error: crate::Error) -> Self {
        Self { error, hint: None }
    }
}

impl From<anyhow::Error> for Failure {
    fn from(error: anyhow::Error) -> Self {
        crate::Error::from(error).into()
    }
}

impl Serialize for Failure {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        FailureWire {
            body: self.body(),
            exit_code: self.exit_code(),
            hint: self.hint(),
        }
        .serialize(serializer)
    }
}

// The flat wire shape: `{"error","message","exit-code","hint"?}`.
#[derive(Serialize)]
#[serde(rename_all = "kebab-case")]
struct FailureWire<'a> {
    #[serde(flatten)]
    body: ErrorBody,
    exit_code: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    hint: Option<&'a str>,
}

type HintFn<'a> = Box<dyn Fn(&crate::Error) -> Option<Cow<'static, str>> + Send + Sync + 'a>;

/// The command projector: decode → [`Client::call`] → encode onto a
/// [`Response`], with the [`Failure`] envelope on stderr.
pub struct Command<'a, P> {
    client: &'a Client<P>,
    metadata: &'a Metadata,
    format: Format,
    hint: Option<HintFn<'a>>,
}

impl<'a, P: Send + Sync + 'static> Command<'a, P> {
    /// Project verbs over `client` with `metadata`, encoding bodies as
    /// `format`.
    #[must_use]
    pub const fn new(client: &'a Client<P>, metadata: &'a Metadata, format: Format) -> Self {
        Self {
            client,
            metadata,
            format,
            hint: None,
        }
    }

    /// Attach a remedy-hint fn consulted for every failure that carries no
    /// hint of its own.
    #[must_use]
    pub fn hints(
        mut self, hint: impl Fn(&crate::Error) -> Option<Cow<'static, str>> + Send + Sync + 'a,
    ) -> Self {
        self.hint = Some(Box::new(hint));
        self
    }

    /// Run one verb: `decode` the input, invoke `handler` through the
    /// client, and encode the output (`render` is its text form).
    pub async fn call<F, I, D, R>(&self, handler: F, decode: D, render: R) -> Response
    where
        F: Handler<P, I>,
        F::Output: Serialize,
        F::Error: Into<Failure>,
        D: FnOnce() -> Result<I, crate::Error>,
        R: Fn(&F::Output, &mut dyn fmt::Write) -> fmt::Result,
    {
        let input = match decode() {
            Ok(input) => input,
            Err(error) => return self.refuse(Failure::from(error)),
        };
        match self.client.call(handler, input, self.metadata).await {
            Ok(output) => Response::success(self.format.encode(&output, render).bytes),
            Err(error) => self.refuse(error.into()),
        }
    }

    fn refuse(&self, mut failure: Failure) -> Response {
        if failure.hint.is_none()
            && let Some(hint) = &self.hint
        {
            failure.hint = hint(&failure.error);
        }
        Response::failure(self.format.encode(&failure, Failure::text).bytes, failure.exit_code())
    }
}

impl<P> fmt::Debug for Command<'_, P> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Command")
            .field("format", &self.format)
            .field("hint", &self.hint.is_some())
            .finish_non_exhaustive()
    }
}

#[cfg(all(target_arch = "wasm32", target_os = "wasi"))]
#[doc(hidden)]
pub trait Export<P, M> {
    fn run(self, provider: P) -> impl Future<Output = ()>;
}

#[cfg(all(target_arch = "wasm32", target_os = "wasi"))]
impl<F, Fut, P, E> Export<P, ()> for F
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = E>,
    E: IntoExit,
{
    async fn run(self, _provider: P) {
        execute_wasi(self()).await;
    }
}

#[cfg(all(target_arch = "wasm32", target_os = "wasi", feature = "command"))]
impl<F, P> Export<P, Router<P>> for F
where
    F: FnOnce(P) -> Router<P>,
    P: Send + Sync + 'static,
{
    async fn run(self, provider: P) {
        execute_wasi(async move {
            let router = self(provider);
            router.handle(wasip3::cli::environment::get_arguments()).await
        })
        .await;
    }
}

#[cfg(all(target_arch = "wasm32", target_os = "wasi", feature = "command"))]
impl<F, Fut, P> Export<P, (Router<P>,)> for F
where
    F: FnOnce(P) -> Fut,
    Fut: Future<Output = Router<P>>,
    P: Send + Sync + 'static,
{
    async fn run(self, provider: P) {
        execute_wasi(async move {
            let router = self(provider).await;
            router.handle(wasip3::cli::environment::get_arguments()).await
        })
        .await;
    }
}

/// Execute a guest command entry at the WASI CLI boundary.
///
/// Initializes guest telemetry, awaits `entry`, and flushes telemetry and
/// stdout.
#[cfg(all(target_arch = "wasm32", target_os = "wasi"))]
#[doc(hidden)]
pub async fn execute_wasi<E: IntoExit>(entry: impl Future<Output = E>) {
    // `scope` exports telemetry before an `exit-with-code`, which never returns.
    let result = omnia_wasi_otel::scope(|| entry).await.into_exit();
    // `exit-with-code` exits immediately, meaning stdout never flushes
    let _ = std::io::stdout().flush();
    if let Err(code) = result
        && code != 0
    {
        wasip3::cli::exit::exit_with_code(code);
    }
}

// The environment variable carrying the metadata field `name` under `prefix`.
fn env_key(prefix: &str, name: &str) -> String {
    format!("{prefix}_{}", name.to_ascii_uppercase().replace('-', "_"))
}

fn write_channel(sink: &mut impl Write, bytes: &[u8]) -> Result<(), u8> {
    match sink.write_all(bytes).and_then(|()| sink.flush()) {
        Ok(()) => Ok(()),
        // a reader that has gone away is the consumer's decision, not this command's failure
        Err(error) if error.kind() == ErrorKind::BrokenPipe => Ok(()),
        // anything else is unclassified, so it takes a `ServerError`'s exit
        Err(error) => Err(crate::server_error!("process channel: {error}").exit_code()),
    }
}

cfg_select! {
    feature = "command" => {
        use std::collections::BTreeMap;
        use std::ffi::OsString;
        use std::future::Future;
        use std::marker::PhantomData;
        use std::path::Path;
        use std::pin::Pin;
        use std::sync::Arc;

        pub use clap_complete::Shell;

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
        /// applies no level for that pair however argv splits it).
        ///
        /// ```rust,ignore
        /// #[derive(clap::Parser)]
        /// struct App {
        ///     #[command(flatten)]
        ///     verbosity: omnia_sdk::api::command::Verbosity,
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

        #[derive(clap::Args)]
        #[command(about = None, long_about = None)]
        struct Globals {
            /// Output format for every verb.
            #[arg(long, default_value = "text", global = true)]
            format: Format,

            #[command(flatten)]
            verbosity: Verbosity,
        }

        type DispatchFuture<'a> = Pin<Box<dyn Future<Output = Response> + Send + 'a>>;

        trait ErasedRoute<P: Send + Sync + 'static>: Send + Sync {
            fn dispatch<'a>(
                &'a self, matches: &'a clap::ArgMatches, command: &'a Command<'a, P>,
            ) -> DispatchFuture<'a>;
        }

        struct Route<P, F, I, D, R> {
            handler: F,
            decode: D,
            render: R,
            marker: PhantomData<fn(P) -> I>,
        }

        impl<P, F, I, D, R> ErasedRoute<P> for Route<P, F, I, D, R>
        where
            P: Send + Sync + 'static,
            F: Handler<P, I>,
            F::Output: Serialize,
            F::Error: Into<Failure>,
            I: Send + 'static,
            D: Fn(&clap::ArgMatches) -> Result<I, crate::Error> + Send + Sync + 'static,
            R: Fn(&F::Output, &mut dyn fmt::Write) -> fmt::Result + Send + Sync + 'static,
        {
            fn dispatch<'a>(
                &'a self, matches: &'a clap::ArgMatches, command: &'a Command<'a, P>,
            ) -> DispatchFuture<'a> {
                Box::pin(async move {
                    command
                        .call(self.handler.clone(), || (self.decode)(matches), &self.render)
                        .await
                })
            }
        }

        struct ErasedCommand<P: Send + Sync + 'static> {
            command: clap::Command,
            route: Arc<dyn ErasedRoute<P>>,
        }

        /// Subcommand routes awaiting the client that dispatches them.
        ///
        /// Two routes for one subcommand name are refused when they meet,
        /// whether through [`route`](Self::route),
        /// [`route_with`](Self::route_with), [`merge`](Self::merge),
        /// `Commands::from([..])`, or [`map`](Self::map).
        #[must_use = "commands dispatch nothing until closed into a `Router`"]
        pub struct Commands<P: Send + Sync + 'static> {
            routes: BTreeMap<String, ErasedCommand<P>>,
        }

        impl<P: Send + Sync + 'static> Default for Commands<P> {
            fn default() -> Self {
                Self::new()
            }
        }

        impl<P: Send + Sync + 'static> Commands<P> {
            /// Create an empty set of subcommand routes.
            pub const fn new() -> Self {
                Self {
                    routes: BTreeMap::new(),
                }
            }

            /// Register one handler for one subcommand name, decoding its
            /// arguments through [`clap::Args`].
            ///
            /// # Panics
            ///
            /// Panics when `name` is empty or already registered.
            pub fn route<F, I, R>(self, name: impl Into<String>, handler: F, render: R) -> Self
            where
                F: Handler<P, I>,
                F::Output: Serialize,
                F::Error: Into<Failure>,
                I: clap::Args + Send + 'static,
                R: Fn(&F::Output, &mut dyn fmt::Write) -> fmt::Result + Send + Sync + 'static,
            {
                let name = name.into();
                assert!(!name.is_empty(), "command name cannot be empty");
                let command = I::augment_args(clap::Command::new(name));
                self.route_with(
                    command,
                    handler,
                    |matches| {
                        I::from_arg_matches(matches).map_err(|error| crate::bad_request!("{error}"))
                    },
                    render,
                )
            }

            /// Register one handler for one [`clap::Command`] subcommand with a
            /// decoder over its [`clap::ArgMatches`].
            ///
            /// # Panics
            ///
            /// Panics when the subcommand name is empty or already registered.
            pub fn route_with<F, I, D, R>(
                self, command: clap::Command, handler: F, decode: D, render: R,
            ) -> Self
            where
                F: Handler<P, I>,
                F::Output: Serialize,
                F::Error: Into<Failure>,
                I: Send + 'static,
                D: Fn(&clap::ArgMatches) -> Result<I, crate::Error> + Send + Sync + 'static,
                R: Fn(&F::Output, &mut dyn fmt::Write) -> fmt::Result + Send + Sync + 'static,
            {
                let name = command.get_name().to_owned();
                let route = Route::<P, F, I, D, R> {
                    handler,
                    decode,
                    render,
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

            /// Take every subcommand route of `other`.
            ///
            /// # Panics
            ///
            /// Panics when a subcommand name of `other` is already registered.
            pub fn merge(self, other: Self) -> Self {
                other
                    .routes
                    .into_iter()
                    .fold(self, |routes, (name, entry)| routes.insert(name, entry))
            }

            /// Re-key every subcommand route through `map`.
            ///
            /// # Panics
            ///
            /// Panics when `map` yields an empty name or one name for two routes.
            pub fn map(self, mut map: impl FnMut(&str) -> String) -> Self {
                self.routes.into_iter().fold(Self::new(), |routes, (name, mut entry)| {
                    let mapped = map(&name);
                    entry.command = entry.command.name(mapped.clone());
                    routes.insert(mapped, entry)
                })
            }

            /// Close the commands over `client` as the [`Router`] that dispatches them.
            pub const fn with_state(self, client: Client<P>) -> Router<P> {
                Router {
                    client,
                    routes: self,
                    name: None,
                    version: None,
                    about: None,
                    env_prefix: None,
                    hint: None,
                }
            }

            fn insert(mut self, name: String, entry: ErasedCommand<P>) -> Self {
                assert!(!name.is_empty(), "command name cannot be empty");
                assert!(!self.routes.contains_key(&name), "duplicate command `{name}`");
                self.routes.insert(name, entry);
                self
            }
        }

        impl<P: Send + Sync + 'static, const N: usize> From<[Self; N]> for Commands<P> {
            fn from(parts: [Self; N]) -> Self {
                parts.into_iter().reduce(Self::merge).unwrap_or_default()
            }
        }

        /// A subcommand router backed by a [`Client`].
        #[must_use = "a router dispatches nothing until it handles argv"]
        pub struct Router<P: Send + Sync + 'static> {
            client: Client<P>,
            routes: Commands<P>,
            name: Option<String>,
            version: Option<String>,
            about: Option<String>,
            env_prefix: Option<String>,
            hint: Option<HintFn<'static>>,
        }

        impl<P: Send + Sync + 'static> Router<P> {
            /// Create an empty subcommand router backed by a client.
            pub const fn new(client: Client<P>) -> Self {
                Commands::new().with_state(client)
            }

            /// Set the binary name shown in usage and help text.
            ///
            /// Defaults to `argv[0]` (or `"app"` when `argv` is empty).
            pub fn name(mut self, name: impl Into<String>) -> Self {
                self.name = Some(name.into());
                self
            }

            /// Set the version string answered for `--version`.
            pub fn version(mut self, version: impl Into<String>) -> Self {
                self.version = Some(version.into());
                self
            }

            /// Set the description shown at the top of `--help`.
            pub fn about(mut self, about: impl Into<String>) -> Self {
                self.about = Some(about.into());
                self
            }

            /// Set the environment-variable prefix [`handle`](Self::handle)
            /// passes to [`Metadata::from_env`].
            ///
            /// Defaults to the binary name in uppercase with `-` replaced by `_`.
            pub fn env_prefix(mut self, prefix: impl Into<String>) -> Self {
                self.env_prefix = Some(prefix.into());
                self
            }

            /// Attach a remedy-hint fn consulted for every failure that carries
            /// no hint of its own.
            pub fn hints(
                mut self,
                hint: impl Fn(&crate::Error) -> Option<Cow<'static, str>> + Send + Sync + 'static,
            ) -> Self {
                self.hint = Some(Box::new(hint));
                self
            }

            /// Register one handler for one subcommand name, decoding its
            /// arguments through [`clap::Args`].
            ///
            /// # Panics
            ///
            /// Panics when `name` is empty or already registered.
            pub fn route<F, I, R>(mut self, name: impl Into<String>, handler: F, render: R) -> Self
            where
                F: Handler<P, I>,
                F::Output: Serialize,
                F::Error: Into<Failure>,
                I: clap::Args + Send + 'static,
                R: Fn(&F::Output, &mut dyn fmt::Write) -> fmt::Result + Send + Sync + 'static,
            {
                self.routes = self.routes.route(name, handler, render);
                self
            }

            /// Register one handler for one [`clap::Command`] subcommand with a
            /// decoder over its [`clap::ArgMatches`].
            ///
            /// # Panics
            ///
            /// Panics when the subcommand name is empty or already registered.
            pub fn route_with<F, I, D, R>(
                mut self, command: clap::Command, handler: F, decode: D, render: R,
            ) -> Self
            where
                F: Handler<P, I>,
                F::Output: Serialize,
                F::Error: Into<Failure>,
                I: Send + 'static,
                D: Fn(&clap::ArgMatches) -> Result<I, crate::Error> + Send + Sync + 'static,
                R: Fn(&F::Output, &mut dyn fmt::Write) -> fmt::Result + Send + Sync + 'static,
            {
                self.routes = self.routes.route_with(command, handler, decode, render);
                self
            }

            /// Take every subcommand route of `commands`.
            ///
            /// # Panics
            ///
            /// Panics when a subcommand name of `commands` is already registered.
            pub fn merge(mut self, commands: Commands<P>) -> Self {
                self.routes = self.routes.merge(commands);
                self
            }

            /// The subcommand names registered on this router, in order.
            pub fn commands(&self) -> impl Iterator<Item = &str> {
                self.routes.routes.keys().map(String::as_str)
            }

            /// Generate a shell-completion script for this router's command grammar.
            #[must_use]
            pub fn completions(&self, shell: Shell) -> Response {
                let name = self.name.as_deref().unwrap_or("app");
                let mut command = self.build_command(name);
                let mut out = Vec::new();
                clap_complete::generate(shell, &mut command, name, &mut out);
                Response::success(out)
            }

            /// Parse `argv`, read [`Metadata::from_env`], and dispatch the matched subcommand.
            pub async fn handle(
                &self, argv: impl IntoIterator<Item: Into<OsString> + Clone>,
            ) -> Response {
                let argv: Vec<OsString> = argv.into_iter().map(Into::into).collect();
                let bin = self.bin_name(&argv);
                let prefix = self
                    .env_prefix
                    .clone()
                    .unwrap_or_else(|| bin.to_ascii_uppercase().replace('-', "_"));
                let metadata = Metadata::from_env(&prefix);
                self.dispatch_argv(argv, &metadata).await
            }

            /// Parse `argv` and dispatch the matched subcommand with explicit `metadata`.
            pub async fn handle_with(
                &self, argv: impl IntoIterator<Item: Into<OsString> + Clone>, metadata: &Metadata,
            ) -> Response {
                let argv: Vec<OsString> = argv.into_iter().map(Into::into).collect();
                self.dispatch_argv(argv, metadata).await
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

            fn build_command(&self, bin_name: &str) -> clap::Command {
                let mut command = clap::Command::new(bin_name.to_owned())
                    .bin_name(bin_name.to_owned())
                    .subcommand_required(true);
                if let Some(version) = &self.version {
                    command = command.version(version.clone());
                }
                if let Some(about) = &self.about {
                    command = command.about(about.clone());
                }
                command = <Globals as clap::Args>::augment_args(command);
                for entry in self.routes.routes.values() {
                    command = command.subcommand(entry.command.clone());
                }
                command
            }

            async fn dispatch_argv(&self, argv: Vec<OsString>, metadata: &Metadata) -> Response {
                let bin = self.bin_name(&argv).to_owned();
                let command = self.build_command(&bin);
                let matches = match command.try_get_matches_from(argv) {
                    Ok(matches) => matches,
                    Err(error) if error.use_stderr() => return Response::usage(&error),
                    Err(display) => return Response::success(display.render().to_string()),
                };
                let globals = <Globals as clap::FromArgMatches>::from_arg_matches(&matches)
                    .expect("globals augmented on command");
                let (name, sub_matches) =
                    matches.subcommand().expect("subcommand_required is enabled");
                let entry = self.routes.routes.get(name).expect("matched subcommand is registered");
                let mut projector = Command::new(&self.client, metadata, globals.format);
                if let Some(hint) = &self.hint {
                    projector = projector.hints(|error| hint(error));
                }
                entry.route.dispatch(sub_matches, &projector).await
            }
        }

        /// Parse argv into the grammar `A`, classifying clap's own outcomes.
        #[must_use]
        pub fn parse<A: clap::Parser>(
            argv: impl IntoIterator<Item: Into<OsString> + Clone>,
        ) -> Parsed<A> {
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
    }
    _ => {}
}

impl Metadata {
    /// Build metadata from the process environment: `<PREFIX>_REQUEST_ID`,
    /// `<PREFIX>_CORRELATION_ID`, `<PREFIX>_CAUSATION_ID`, and
    /// `<PREFIX>_TENANT`.
    ///
    /// The command line's carrier for the values HTTP reads from headers; a
    /// missing request id is minted as on every transport.
    #[must_use]
    pub fn from_env(prefix: &str) -> Self {
        Self::from_lookup(|name| std::env::var(env_key(prefix, name)).ok())
    }
}
