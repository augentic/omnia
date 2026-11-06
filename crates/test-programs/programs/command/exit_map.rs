//! The exit map through the real runtime: each verb's error class leaves the
//! `command::export!` boundary as its mapped exit status (`bad` 1, `missing` 2,
//! `upstream` 4, `ok` 0), an unknown verb is a clap usage error at
//! `USAGE_EXIT`, and `--version` and `--help` answer with what the export
//! stamped from the crate.

#![cfg(target_arch = "wasm32")]

use omnia_sdk::api::{Client, Context, Handlers, Router, command};
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

fn command_router(provider: Provider) -> Router<Provider> {
    Handlers::new()
        .command("ok", ok)
        .command("bad", bad)
        .command("missing", missing)
        .command("upstream", upstream)
        .with_state(Client::new(provider))
        .name("exit-map")
}

command::export!(command_router, Provider);
