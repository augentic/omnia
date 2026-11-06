//! # CLI Command Wasm Guest
//!
//! A `wasi:cli/command` reactor on the command router: `Commands` registers
//! each subcommand over its `clap::Args` input, `Router` dispatches argv
//! through `Client::call` onto a `Response`, and `command::export!` binds
//! `command_router` as the `wasi:cli/run` export.
//!
//! The module is `#[cfg(target_arch = "wasm32")]`-guarded because examples
//! also compile for the host triple, where `wasip3` is unavailable.

#![cfg(target_arch = "wasm32")]

use std::collections::BTreeMap;
use std::fmt;

use clap::{Args, ValueEnum};
use omnia_sdk::api::command::{self, Commands, Router};
use omnia_sdk::api::{Client, Context};
use omnia_sdk::{Error, bad_gateway, bad_request, not_found, server_error};
use serde::Serialize;

#[derive(Args)]
struct GreetInput {
    /// Who to greet
    #[arg(default_value = "world")]
    name: String,
}

#[derive(Args)]
struct AddInput {
    /// Integers to sum
    numbers: Vec<i64>,
}

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

#[derive(Serialize)]
struct Sum {
    sum: i64,
}

async fn greet(input: GreetInput, context: Context<Provider>) -> Result<Greeting, Error> {
    Ok(Greeting {
        greeting: format!("{}, {}!", context.provider().greeting, input.name),
    })
}

async fn add(input: AddInput, _context: Context<Provider>) -> Result<Sum, Error> {
    Ok(Sum {
        sum: input.numbers.iter().sum(),
    })
}

async fn env((): (), _context: Context<Provider>) -> Result<BTreeMap<String, String>, Error> {
    Ok(std::env::vars().collect())
}

async fn fail(input: FailInput, _context: Context<Provider>) -> Result<(), Error> {
    Err(match input.class {
        ErrorClass::BadRequest => bad_request!("failing as a bad request"),
        ErrorClass::NotFound => not_found!("failing as not found"),
        ErrorClass::ServerError => server_error!("failing as a server error"),
        ErrorClass::BadGateway => bad_gateway!("failing as a bad gateway"),
    })
}

fn render_greeting(greeting: &Greeting, out: &mut dyn fmt::Write) -> fmt::Result {
    writeln!(out, "{}", greeting.greeting)
}

fn render_sum(sum: &Sum, out: &mut dyn fmt::Write) -> fmt::Result {
    writeln!(out, "{}", sum.sum)
}

fn render_env(vars: &BTreeMap<String, String>, out: &mut dyn fmt::Write) -> fmt::Result {
    vars.iter().try_for_each(|(key, value)| writeln!(out, "{key}={value}"))
}

fn render_nothing((): &(), _out: &mut dyn fmt::Write) -> fmt::Result {
    Ok(())
}

fn command_router(provider: Provider) -> Router<Provider> {
    Commands::new()
        .route("greet", greet, render_greeting)
        .route("add", add, render_sum)
        .route("env", env, render_env)
        .route("fail", fail, render_nothing)
        .with_state(Client::new(provider))
        .name("cli")
        .version(env!("CARGO_PKG_VERSION"))
        .about("Omnia wasi:cli/command example")
}

command::export!(command_router, Provider { greeting: "Hello" });
