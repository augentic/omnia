//! The command façade: argv in, buffered channels and an exit status out.
//!
//! A subcommand is one more handler on a [`Handlers`](crate::api::Handlers).
//! [`Handlers::command`](crate::api::Handlers::command) registers it beside
//! the paths and topics, decoding its arguments through [`clap::Args`];
//! [`Handlers::command_with`](crate::api::Handlers::command_with) takes a
//! decoder over the [`clap::ArgMatches`] instead. The closed
//! [`Router`](crate::api::Router) [`run`](crate::api::Router::run)s argv:
//!
//! - the verb's [`Handler`] runs through [`Client::call`]
//! - a success body is encoded onto stdout in the selected [`Format`], its
//!   text form the output's [`Render`]
//! - a [`Failure`] envelope goes to stderr at
//!   [`Error::exit_code`](crate::Error::exit_code)
//! - a clap usage error exits [`USAGE_EXIT`]
//!
//! [`export!`] binds the router fn (or a zero-argument `async fn` entry) as
//! the `wasi:cli/run` export, with the crate's version and description as
//! the program's defaults, and [`IntoExit`] writes the channels at that
//! boundary.
//!
//! ```rust,ignore
//! use std::fmt;
//!
//! use clap::Args;
//! use omnia_sdk::api::command::{self, Render};
//! use omnia_sdk::api::{Client, Context, Handlers, Router};
//! use serde::Serialize;
//!
//! /// Greet someone by name.
//! #[derive(Args)]
//! struct Greet {
//!     name: String,
//! }
//!
//! #[derive(Serialize)]
//! struct Greeting {
//!     text: String,
//! }
//!
//! impl Render for Greeting {
//!     fn render(&self, out: &mut dyn fmt::Write) -> fmt::Result {
//!         writeln!(out, "{}", self.text)
//!     }
//! }
//!
//! async fn greet<P>(_context: Context<P>, input: Greet) -> omnia_sdk::Result<Greeting> {
//!     Ok(Greeting { text: format!("hello, {}", input.name) })
//! }
//!
//! pub fn command_router<P: Send + Sync + 'static>(provider: P) -> Router<P> {
//!     Handlers::new().command("greet", greet).with_state(Client::new(provider))
//! }
//!
//! command::export!(command_router);
//! ```

use std::borrow::Cow;
use std::fmt;
use std::io::{ErrorKind, Write};
use std::sync::Arc;

use serde::{Serialize, Serializer};

use crate::api::{Client, ErrorBody, Handler, Metadata};

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
                    $crate::api::command::Export::run(
                        $entry,
                        $provider,
                        $crate::api::command::Package {
                            version: ::core::env!("CARGO_PKG_VERSION"),
                            description: ::core::env!("CARGO_PKG_DESCRIPTION"),
                        },
                    )
                    .await;
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
/// provider expression passed as a second argument) and runs the WASI
/// command-line arguments through the resulting
/// [`Router`](crate::api::Router), or awaits a zero-argument `async fn`
/// returning [`IntoExit`], inside the guest telemetry scope. The router's
/// program answers `--version` with the invoking crate's `CARGO_PKG_VERSION`
/// and opens `--help` with its `CARGO_PKG_DESCRIPTION` unless
/// [`Router::version`](crate::api::Router::version) or
/// [`Router::about`](crate::api::Router::about) set them. Expands to nothing
/// on native targets, so the same crate's tests build the router natively.
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

/// The wire format a handler output is encoded into: the body's [`Render`]
/// form as text, its [`Serialize`] form as JSON.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[cfg_attr(feature = "command", derive(clap::ValueEnum))]
pub enum Format {
    /// Human-readable text.
    Text,

    /// Pretty-printed JSON with a trailing newline.
    Json,
}

impl Format {
    /// Encode a body: its [`Render`] form for [`Format::Text`], its
    /// [`Serialize`] form for [`Format::Json`].
    ///
    /// # Panics
    ///
    /// Panics if `body` fails to render (a `String` sink never does) or to
    /// serialize; both are invariant violations for a derived DTO encoded
    /// into memory, not runtime conditions.
    pub fn encode<T: Serialize + Render>(self, body: &T) -> Encoded {
        match self {
            Self::Text => {
                let mut out = String::new();
                body.render(&mut out).expect("a String sink never fails");
                Encoded {
                    bytes: out.into_bytes(),
                    media_type: "text/plain; charset=utf-8",
                }
            }
            Self::Json => {
                let mut bytes = serde_json::to_vec_pretty(body).expect("a derived DTO serializes");
                bytes.push(b'\n');
                Encoded {
                    bytes,
                    media_type: "application/json",
                }
            }
        }
    }
}

/// An encoded body with its media type.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Encoded {
    /// The encoded body bytes.
    pub bytes: Vec<u8>,

    /// The media type describing `bytes`.
    pub media_type: &'static str,
}

/// The text form of a body, for [`Format::Text`].
///
/// [`Serialize`] gives a handler output its JSON form; this gives it the
/// line-oriented one a command writes to stdout. `()` renders nothing and
/// `String` renders itself followed by a newline.
pub trait Render {
    /// Write the text form to `out`.
    ///
    /// # Errors
    ///
    /// Propagates the sink's formatting failure.
    fn render(&self, out: &mut dyn fmt::Write) -> fmt::Result;
}

impl Render for () {
    fn render(&self, _out: &mut dyn fmt::Write) -> fmt::Result {
        Ok(())
    }
}

impl Render for String {
    fn render(&self, out: &mut dyn fmt::Write) -> fmt::Result {
        writeln!(out, "{self}")
    }
}

/// The failure envelope a command reports: the error, its exit status, and
/// an optional remedy hint.
///
/// Its text form ([`Render`]) is `error[<code>]: <message>` and an optional
/// `hint:` line; its JSON form ([`Serialize`]) is the flat
/// `{"error","message","exit-code","hint"?}`.
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
}

impl Render for Failure {
    fn render(&self, out: &mut dyn fmt::Write) -> fmt::Result {
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

type HintFn<'a> = Arc<dyn Fn(&crate::Error) -> Option<Cow<'static, str>> + Send + Sync + 'a>;

/// The command projector: [`Client::call`] → encode onto a [`Response`],
/// with the [`Failure`] envelope on stderr.
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
        self.hint = Some(Arc::new(hint));
        self
    }

    /// Run one verb: invoke `handler` on `input` through the client and
    /// encode the output in the projector's [`Format`].
    pub async fn call<F, I>(&self, handler: F, input: I) -> Response
    where
        F: Handler<P, I>,
        F::Output: Serialize + Render,
        F::Error: Into<Failure>,
    {
        match self.client.call(handler, input, self.metadata).await {
            Ok(output) => Response::success(self.format.encode(&output).bytes),
            Err(error) => self.refuse(error.into()),
        }
    }

    fn refuse(&self, mut failure: Failure) -> Response {
        if failure.hint.is_none()
            && let Some(hint) = &self.hint
        {
            failure.hint = hint(&failure.error);
        }
        Response::failure(self.format.encode(&failure).bytes, failure.exit_code())
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
pub struct Package {
    pub version: &'static str,
    pub description: &'static str,
}

#[cfg(all(target_arch = "wasm32", target_os = "wasi"))]
#[doc(hidden)]
pub trait Export<P, M> {
    fn run(self, provider: P, package: Package) -> impl Future<Output = ()>;
}

#[cfg(all(target_arch = "wasm32", target_os = "wasi"))]
impl<F, Fut, P, E> Export<P, ()> for F
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = E>,
    E: IntoExit,
{
    async fn run(self, _provider: P, _package: Package) {
        execute_wasi(self()).await;
    }
}

#[cfg(all(target_arch = "wasm32", target_os = "wasi", feature = "command"))]
impl<F, P> Export<P, crate::api::Router<P>> for F
where
    F: FnOnce(P) -> crate::api::Router<P>,
    P: Send + Sync + 'static,
{
    async fn run(self, provider: P, package: Package) {
        execute_wasi(async move {
            let mut router = self(provider);
            router.program.package(&package);
            router.run(wasip3::cli::environment::get_arguments()).await
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

        pub use clap_complete::Shell;
        use tracing::Instrument as _;

        use crate::api::DecodeError;

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
        /// [`Router`](crate::api::Router) declares them itself.
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

        pub(crate) struct Commands<P: Send + Sync + 'static> {
            routes: BTreeMap<String, ErasedCommand<P>>,
        }

        impl<P: Send + Sync + 'static> Commands<P> {
            pub(crate) const fn new() -> Self {
                Self {
                    routes: BTreeMap::new(),
                }
            }

            pub(crate) fn route<F, I>(self, name: impl Into<String>, handler: F) -> Self
            where
                F: Handler<P, I>,
                F::Output: Serialize + Render,
                F::Error: Into<Failure>,
                I: clap::Args + Send + 'static,
            {
                let name = name.into();
                assert!(!name.is_empty(), "command name cannot be empty");
                let command = I::augment_args(clap::Command::new(name));
                self.route_with(command, handler, |matches| {
                    I::from_arg_matches(matches)
                        .map_err(|error| DecodeError::new(error.to_string()))
                })
            }

            pub(crate) fn route_with<F, I, D>(
                self, command: clap::Command, handler: F, decode: D,
            ) -> Self
            where
                F: Handler<P, I>,
                F::Output: Serialize + Render,
                F::Error: Into<Failure>,
                I: Send + 'static,
                D: Fn(&clap::ArgMatches) -> Result<I, DecodeError> + Send + Sync + 'static,
            {
                let name = command.get_name().to_owned();
                let route = Route::<P, F, I, D> {
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

            pub(crate) fn merge(self, other: Self) -> Self {
                other
                    .routes
                    .into_iter()
                    .fold(self, |routes, (name, entry)| routes.insert(name, entry))
            }

            pub(crate) fn commands(&self) -> impl Iterator<Item = &str> {
                self.routes.keys().map(String::as_str)
            }

            fn insert(mut self, name: String, entry: ErasedCommand<P>) -> Self {
                assert!(!name.is_empty(), "command name cannot be empty");
                assert!(!self.routes.contains_key(&name), "duplicate command `{name}`");
                self.routes.insert(name, entry);
                self
            }
        }

        impl<P: Send + Sync + 'static> Clone for Commands<P> {
            fn clone(&self) -> Self {
                Self {
                    routes: self.routes.clone(),
                }
            }
        }

        impl<P: Send + Sync + 'static> fmt::Debug for Commands<P> {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.debug_set().entries(self.routes.keys()).finish()
            }
        }

        struct ErasedCommand<P: Send + Sync + 'static> {
            command: clap::Command,
            route: Arc<dyn ErasedRoute<P>>,
        }

        impl<P: Send + Sync + 'static> Clone for ErasedCommand<P> {
            fn clone(&self) -> Self {
                Self {
                    command: self.command.clone(),
                    route: Arc::clone(&self.route),
                }
            }
        }

        type DispatchFuture<'a> = Pin<Box<dyn Future<Output = Response> + Send + 'a>>;

        trait ErasedRoute<P: Send + Sync + 'static>: Send + Sync {
            fn dispatch<'a>(
                &'a self, matches: &'a clap::ArgMatches, command: &'a Command<'a, P>,
            ) -> DispatchFuture<'a>;
        }

        struct Route<P, F, I, D> {
            handler: F,
            decode: D,
            marker: PhantomData<fn(P) -> I>,
        }

        impl<P, F, I, D> ErasedRoute<P> for Route<P, F, I, D>
        where
            P: Send + Sync + 'static,
            F: Handler<P, I>,
            F::Output: Serialize + Render,
            F::Error: Into<Failure>,
            I: Send + 'static,
            D: Fn(&clap::ArgMatches) -> Result<I, DecodeError> + Send + Sync + 'static,
        {
            fn dispatch<'a>(
                &'a self, matches: &'a clap::ArgMatches, command: &'a Command<'a, P>,
            ) -> DispatchFuture<'a> {
                Box::pin(async move {
                    match (self.decode)(matches) {
                        Ok(input) => command.call(self.handler.clone(), input).await,
                        Err(error) => command.refuse(crate::bad_request!("{error}").into()),
                    }
                })
            }
        }

        // The subcommands a closed router runs, with the program identity
        // clap shows around them.
        pub(crate) struct Program<P: Send + Sync + 'static> {
            pub(crate) commands: Commands<P>,
            pub(crate) name: Option<String>,
            pub(crate) version: Option<String>,
            pub(crate) about: Option<String>,
            pub(crate) env_prefix: Option<String>,
            pub(crate) hint: Option<HintFn<'static>>,
        }

        impl<P: Send + Sync + 'static> Program<P> {
            pub(crate) const fn new(commands: Commands<P>) -> Self {
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
            #[cfg(all(target_arch = "wasm32", target_os = "wasi"))]
            pub(crate) fn package(&mut self, package: &Package) {
                self.version.get_or_insert_with(|| package.version.to_owned());
                if !package.description.is_empty() {
                    self.about.get_or_insert_with(|| package.description.to_owned());
                }
            }

            pub(crate) fn completions(&self, shell: Shell) -> Response {
                let name = self.name.as_deref().unwrap_or("app");
                let mut command = self.build_command(name);
                let mut out = Vec::new();
                clap_complete::generate(shell, &mut command, name, &mut out);
                Response::success(out)
            }

            pub(crate) async fn run(
                &self, client: &Client<P>, argv: impl IntoIterator<Item: Into<OsString>>,
            ) -> Response {
                let argv: Vec<OsString> = argv.into_iter().map(Into::into).collect();
                let bin = self.bin_name(&argv);
                let prefix = self
                    .env_prefix
                    .clone()
                    .unwrap_or_else(|| bin.to_ascii_uppercase().replace('-', "_"));
                let metadata = Metadata::from_env(&prefix);
                self.dispatch(client, argv, &metadata).await
            }

            pub(crate) async fn run_with(
                &self, client: &Client<P>, argv: impl IntoIterator<Item: Into<OsString>>,
                metadata: &Metadata,
            ) -> Response {
                let argv: Vec<OsString> = argv.into_iter().map(Into::into).collect();
                self.dispatch(client, argv, metadata).await
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

            async fn dispatch(
                &self, client: &Client<P>, argv: Vec<OsString>, metadata: &Metadata,
            ) -> Response {
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
                let entry =
                    self.commands.routes.get(name).expect("matched subcommand is registered");
                let mut projector = Command::new(client, metadata, globals.format);
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

        impl<P: Send + Sync + 'static> Clone for Program<P> {
            fn clone(&self) -> Self {
                Self {
                    commands: self.commands.clone(),
                    name: self.name.clone(),
                    version: self.version.clone(),
                    about: self.about.clone(),
                    env_prefix: self.env_prefix.clone(),
                    hint: self.hint.clone(),
                }
            }
        }

        impl<P: Send + Sync + 'static> fmt::Debug for Program<P> {
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
