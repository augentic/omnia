//! # CLI Command Wasm Guest
//!
//! A `wasi:cli/command` reactor on the one router: `Handlers::command`
//! registers each subcommand over its `clap::Args` input, `Router::run`
//! dispatches argv (decode → `Client::call` → encode) onto a `Response`, and
//! `command::export!` binds `command_router` as the `wasi:cli/run` export.
//!
//! The module is `#[cfg(target_arch = "wasm32")]`-guarded because examples
//! also compile for the host triple, where `wasip3` is unavailable.

#![cfg(target_arch = "wasm32")]

use std::collections::BTreeMap;
use std::fmt;

use clap::{Args, ValueEnum};
use omnia_sdk::api::{Client, Context, Handlers, Render, Router, command};
use omnia_sdk::{Error, bad_gateway, bad_request, not_found, server_error};
use serde::Serialize;

/// Greet someone by name.
#[derive(Args)]
struct GreetInput {
    /// Who to greet
    #[arg(default_value = "world")]
    name: String,
}

/// Sum integers.
#[derive(Args)]
struct AddInput {
    /// Integers to sum
    numbers: Vec<i64>,
}

/// Print the guest's environment.
#[derive(Args)]
struct EnvInput;

/// Fail with an error class, to observe the exit map.
#[derive(Args)]
struct FailInput {
    /// The error class to fail with
    #[arg(default_value = "server-error")]
    class: ErrorClass,
}

#[derive(Clone, Copy, ValueEnum)]
enum ErrorClass {
    /// Exit 1
    BadRequest,
    /// Exit 2
    NotFound,
    /// Exit 3
    ServerError,
    /// Exit 4
    BadGateway,
}

struct Provider {
    greeting: &'static str,
}

#[derive(Serialize)]
struct Greeting {
    greeting: String,
}

impl Render for Greeting {
    fn render(&self, out: &mut dyn fmt::Write) -> fmt::Result {
        writeln!(out, "{}", self.greeting)
    }
}

#[derive(Serialize)]
struct Sum {
    sum: i64,
}

impl Render for Sum {
    fn render(&self, out: &mut dyn fmt::Write) -> fmt::Result {
        writeln!(out, "{}", self.sum)
    }
}

#[derive(Serialize)]
#[serde(transparent)]
struct Env(BTreeMap<String, String>);

impl Render for Env {
    fn render(&self, out: &mut dyn fmt::Write) -> fmt::Result {
        self.0.iter().try_for_each(|(key, value)| writeln!(out, "{key}={value}"))
    }
}

async fn greet(context: Context<Provider>, input: GreetInput) -> Result<Greeting, Error> {
    Ok(Greeting {
        greeting: format!("{}, {}!", context.provider().greeting, input.name),
    })
}

async fn add(_context: Context<Provider>, input: AddInput) -> Result<Sum, Error> {
    Ok(Sum {
        sum: input.numbers.iter().sum(),
    })
}

async fn env(_context: Context<Provider>, EnvInput: EnvInput) -> Result<Env, Error> {
    Ok(Env(std::env::vars().collect()))
}

async fn fail(_context: Context<Provider>, input: FailInput) -> Result<(), Error> {
    Err(match input.class {
        ErrorClass::BadRequest => bad_request!("failing as a bad request"),
        ErrorClass::NotFound => not_found!("failing as not found"),
        ErrorClass::ServerError => server_error!("failing as a server error"),
        ErrorClass::BadGateway => bad_gateway!("failing as a bad gateway"),
    })
}

fn command_router(provider: Provider) -> Router<Provider> {
    Handlers::new()
        .command("greet", greet)
        .command("add", add)
        .command("env", env)
        .command("fail", fail)
        .with_state(Client::new(provider))
        .name("cli")
        .about("Omnia wasi:cli/command example")
}

command::export!(command_router, Provider { greeting: "Hello" });
