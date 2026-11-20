//! The exit map through the real runtime: each verb's error class leaves the
//! `command::export!` boundary as its mapped exit status (`bad` 1, `missing` 2,
//! `upstream` 4, `taken` 5, `ok` 0), an unknown verb is a clap usage error at
//! `USAGE_EXIT`, and `--version` and `--help` answer with what the export
//! stamped from the crate.

#![cfg(target_arch = "wasm32")]

use omnia_sdk::{Error, Handlers, Router, bad_gateway, bad_request, command, conflict, not_found};

async fn ok((): ()) -> Result<String, Error> {
    Ok("ok".to_owned())
}

async fn bad((): ()) -> Result<(), Error> {
    Err(bad_request!("refused as a bad request"))
}

async fn missing((): ()) -> Result<(), Error> {
    Err(not_found!("refused as not found"))
}

async fn upstream((): ()) -> Result<(), Error> {
    Err(bad_gateway!("refused as a bad gateway"))
}

async fn taken((): ()) -> Result<(), Error> {
    Err(conflict!("refused as a conflict"))
}

fn command_router() -> Router {
    Handlers::new()
        .command("ok", ok)
        .command("bad", bad)
        .command("missing", missing)
        .command("upstream", upstream)
        .command("taken", taken)
        .into_router()
        .name("exit-map")
}

command::export!(command_router);
