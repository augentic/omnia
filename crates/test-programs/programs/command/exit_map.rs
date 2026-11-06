//! The exit map through the real runtime: each verb's error class leaves the
//! `command::export!` boundary as its mapped exit status (`bad` 1, `missing` 2,
//! `upstream` 4, `ok` 0), and an unknown verb is a clap usage error at
//! `USAGE_EXIT`.

#![cfg(target_arch = "wasm32")]

use std::fmt;

use omnia_sdk::api::command::{self, Commands, Router};
use omnia_sdk::api::{Client, Context};
use omnia_sdk::{Error, bad_gateway, bad_request, not_found};

struct Provider;

async fn ok((): (), _context: Context<Provider>) -> Result<String, Error> {
    Ok("ok".to_owned())
}

async fn bad((): (), _context: Context<Provider>) -> Result<(), Error> {
    Err(bad_request!("refused as a bad request"))
}

async fn missing((): (), _context: Context<Provider>) -> Result<(), Error> {
    Err(not_found!("refused as not found"))
}

async fn upstream((): (), _context: Context<Provider>) -> Result<(), Error> {
    Err(bad_gateway!("refused as a bad gateway"))
}

fn render_text(text: &String, out: &mut dyn fmt::Write) -> fmt::Result {
    writeln!(out, "{text}")
}

fn render_nothing((): &(), _out: &mut dyn fmt::Write) -> fmt::Result {
    Ok(())
}

fn command_router(provider: Provider) -> Router<Provider> {
    Commands::new()
        .route("ok", ok, render_text)
        .route("bad", bad, render_nothing)
        .route("missing", missing, render_nothing)
        .route("upstream", upstream, render_nothing)
        .with_state(Client::new(provider))
        .name("exit-map")
}

command::export!(command_router, Provider);
