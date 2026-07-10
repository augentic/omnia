//! # CLI Command Wasm Guest
//!
//! A `wasi:cli/command` reactor with explicit operation routes built by
//! `omnia_guest::api::command`. The guest owns the `wasi:cli/run@0.3.0` export and calls
//! the command adapter once; typed operations remain independent of argv,
//! output, and exit-code policy.
//!
//! The module is `#[cfg(target_arch = "wasm32")]`-guarded because examples
//! also compile for the host triple, where `wasip3` is unavailable.

#![cfg(target_arch = "wasm32")]

use std::convert::Infallible;
use std::error::Error;
use std::fmt;

use clap::Args;
use omnia_guest::api::command::{
    self, App, CommandResponse, NoGlobals, Outcome, Projector, Router,
};
use omnia_guest::api::invoke::{CallContext, Invoker};
use omnia_guest::api::operation::Operation;
use wasip3::exports::cli::run::Guest;

#[derive(Args)]
struct GreetArgs {
    /// Who to greet
    #[arg(default_value = "world")]
    name: String,
}

#[derive(Args)]
struct AddArgs {
    /// Integers to sum
    numbers: Vec<i64>,
}

#[derive(Args)]
struct EnvArgs {}

#[derive(Args)]
struct FailArgs {
    /// Specific exit code to carry through wasi:cli/exit
    code: Option<u8>,
}

struct Provider {
    greeting: &'static str,
}

struct GreetInput {
    name: String,
}

struct AddInput {
    numbers: Vec<i64>,
}

struct EnvInput;

struct FailInput {
    code: Option<u8>,
}

impl From<GreetArgs> for GreetInput {
    fn from(args: GreetArgs) -> Self {
        Self { name: args.name }
    }
}

impl From<AddArgs> for AddInput {
    fn from(args: AddArgs) -> Self {
        Self {
            numbers: args.numbers,
        }
    }
}

impl From<EnvArgs> for EnvInput {
    fn from(_args: EnvArgs) -> Self {
        Self
    }
}

impl From<FailArgs> for FailInput {
    fn from(args: FailArgs) -> Self {
        Self { code: args.code }
    }
}

#[derive(Debug)]
enum CommandError {
    Exit(u8),
    Plain,
}

impl CommandError {
    const fn exit(&self) -> u8 {
        match self {
            Self::Exit(code) => *code,
            Self::Plain => 1,
        }
    }

    const fn message(&self) -> &'static str {
        match self {
            Self::Exit(_) => "",
            Self::Plain => "failing plainly\n",
        }
    }
}

impl fmt::Display for CommandError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.message())
    }
}

impl Error for CommandError {}

struct Greet;
struct Add;
struct Env;
struct Fail;

impl Operation<Provider> for Greet {
    type Error = CommandError;
    type Input = GreetInput;
    type Output = String;

    async fn call(
        input: Self::Input, context: CallContext<'_, Provider>,
    ) -> Result<Self::Output, Self::Error> {
        Ok(format!("{}, {}!\n", context.provider.greeting, input.name))
    }
}

impl Operation<Provider> for Add {
    type Error = CommandError;
    type Input = AddInput;
    type Output = String;

    async fn call(
        input: Self::Input, _context: CallContext<'_, Provider>,
    ) -> Result<Self::Output, Self::Error> {
        Ok(format!("{}\n", input.numbers.iter().sum::<i64>()))
    }
}

impl Operation<Provider> for Env {
    type Error = CommandError;
    type Input = EnvInput;
    type Output = String;

    async fn call(
        _input: Self::Input, _context: CallContext<'_, Provider>,
    ) -> Result<Self::Output, Self::Error> {
        let output = std::env::vars().map(|(key, value)| format!("{key}={value}\n")).collect();
        Ok(output)
    }
}

impl Operation<Provider> for Fail {
    type Error = CommandError;
    type Input = FailInput;
    type Output = String;

    async fn call(
        input: Self::Input, _context: CallContext<'_, Provider>,
    ) -> Result<Self::Output, Self::Error> {
        Err(input.code.map_or(CommandError::Plain, CommandError::Exit))
    }
}

#[derive(Clone, Copy)]
struct Text;

impl Projector<String, CommandError, Infallible, NoGlobals> for Text {
    type Error = Infallible;

    fn project(
        &self, outcome: Outcome<String, CommandError, Infallible>, _globals: &NoGlobals,
    ) -> Result<CommandResponse, Self::Error> {
        Ok(match outcome {
            Outcome::Output(output) => CommandResponse::success(output),
            Outcome::Operation(error) => CommandResponse::failure(error.message(), error.exit()),
            Outcome::Decode(error) => match error {},
        })
    }

    fn project_failure(&self, error: Self::Error, _globals: &NoGlobals) -> CommandResponse {
        match error {}
    }
}

fn router() -> Router<Provider> {
    Router::new(
        App::new("cli").version(env!("CARGO_PKG_VERSION")).about("Omnia wasi:cli/command example"),
        Invoker::new("examples", Provider { greeting: "Hello" }),
    )
    .route(
        ["greet"],
        command::run::<GreetArgs, Greet>().about("Print a greeting").project_with(Text),
    )
    .route(
        ["add"],
        command::run::<AddArgs, Add>()
            .about("Print the sum of the integer arguments")
            .project_with(Text),
    )
    .route(
        ["env"],
        command::run::<EnvArgs, Env>()
            .about("Print the inherited environment, one key=value per line")
            .project_with(Text),
    )
    .route(
        ["fail"],
        command::run::<FailArgs, Fail>()
            .about("Exit with CODE via wasi:cli/exit, or fail plainly (exit 1) without it")
            .project_with(Text),
    )
    .build()
    .expect("command routes are valid")
}

struct Cli;
wasip3::cli::command::export!(Cli);

impl Guest for Cli {
    async fn run() -> Result<(), ()> {
        command::execute_wasi(&router()).await
    }
}
