//! The command façade: argv in, buffered channels and an exit status out.
//!
//! A subcommand is one more handler on a [`Handlers`](crate::api::Handlers).
//! [`Handlers::command`](crate::api::Handlers::command) registers it beside
//! the paths and topics, decoding its arguments through [`clap::Args`];
//! [`Handlers::command_with`](crate::api::Handlers::command_with) takes a
//! decoder over the [`clap::ArgMatches`] instead. The closed
//! [`Router`](crate::api::Router) [`run`](crate::api::Router::run)s argv:
//!
//! - the verb's [`Handler`] runs over the router's provider
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
//! use omnia_sdk::api::{Context, Handlers, Router};
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
//!     Handlers::new().command("greet", greet).with_state(provider)
//! }
//!
//! command::export!(command_router);
//! ```

use std::borrow::Cow;
use std::fmt;
use std::io::{ErrorKind, Write};
use std::sync::Arc;

use serde::{Serialize, Serializer};

use crate::ErrorBody;
use crate::api::{Context, Handler, Metadata};

#[cfg(feature = "command")]
pub(crate) mod program;

#[cfg(feature = "command")]
pub use self::program::{Parsed, Shell, Verbosity, completions, parse};

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
                    $crate::wasi::command::Export::run(
                        $entry,
                        $provider,
                        $crate::wasi::command::Package {
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
/// Calls a router fn with `WasiProvider` (or with the provider expression
/// passed as a second argument) and runs the WASI command-line arguments
/// through the resulting [`Router`](crate::api::Router), or awaits a
/// zero-argument `async fn` returning [`IntoExit`], inside the guest
/// telemetry scope. The router's program answers `--version` with the
/// invoking crate's `CARGO_PKG_VERSION` and opens `--help` with its
/// `CARGO_PKG_DESCRIPTION` unless
/// [`Router::version`](crate::api::Router::version) or
/// [`Router::about`](crate::api::Router::about) set them. Expands to nothing
/// on native targets, where a test or a native host serves the router itself.
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

impl IntoExit for Response {
    fn into_exit(self) -> Result<(), u8> {
        write_channel(&mut std::io::stdout(), &self.stdout)?;
        write_channel(&mut std::io::stderr(), &self.stderr)?;
        if self.exit == 0 { Ok(()) } else { Err(self.exit) }
    }
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

/// The command projector: a verb run over a shared provider, its output
/// encoded onto a [`Response`], with the [`Failure`] envelope on stderr.
pub struct Command<'a, P> {
    provider: &'a Arc<P>,
    metadata: &'a Metadata,
    format: Format,
    hint: Option<HintFn<'a>>,
}

impl<'a, P: Send + Sync + 'static> Command<'a, P> {
    /// Project verbs over `provider` with `metadata`, encoding bodies as
    /// `format`.
    #[must_use]
    pub const fn new(provider: &'a Arc<P>, metadata: &'a Metadata, format: Format) -> Self {
        Self {
            provider,
            metadata,
            format,
            hint: None,
        }
    }

    /// Attach a remedy-hint fn consulted for every failure.
    #[must_use]
    pub fn hints(
        mut self, hint: impl Fn(&crate::Error) -> Option<Cow<'static, str>> + Send + Sync + 'a,
    ) -> Self {
        self.hint = Some(Arc::new(hint));
        self
    }

    /// Run one verb: invoke `handler` on `input` over the provider and
    /// encode the output in the projector's [`Format`].
    ///
    /// A failure is the [`Failure`] envelope over the [`Error`](crate::Error)
    /// that [`Handler::Error`] describes.
    pub async fn call<F, I>(&self, handler: F, input: I) -> Response
    where
        F: Handler<P, I>,
        F::Output: Serialize + Render,
    {
        match handler.call(Context::shared(self.provider, self.metadata), input).await {
            Ok(output) => Response::success(self.format.encode(&output).bytes),
            Err(error) => self.refuse(crate::Error::from_chain(&error)),
        }
    }

    fn refuse(&self, error: crate::Error) -> Response {
        let hint = self.hint.as_ref().and_then(|hint| hint(&error));
        let failure = Failure { error, hint };
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

impl Metadata {
    /// Build metadata from the process environment: `<PREFIX>_REQUEST_ID`,
    /// `<PREFIX>_CORRELATION_ID`, `<PREFIX>_CAUSATION_ID`, `<PREFIX>_TENANT`,
    /// and `<PREFIX>_DEADLINE`.
    ///
    /// The command line's carrier for the values HTTP reads from headers; a
    /// missing request id is minted as on every transport.
    ///
    /// # Panics
    ///
    /// Panics as [`Metadata::from_lookup`] does.
    #[must_use]
    pub fn from_env(prefix: &str) -> Self {
        Self::from_lookup(|name| std::env::var(env_key(prefix, name)).ok())
    }
}

// The environment variable carrying the metadata field `name` under `prefix`.
fn env_key(prefix: &str, name: &str) -> String {
    format!("{prefix}_{}", name.to_ascii_uppercase().replace('-', "_"))
}
