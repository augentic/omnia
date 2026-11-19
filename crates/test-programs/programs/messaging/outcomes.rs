//! A messaging guest whose `omnia_sdk` router fails each topic with its own
//! class of failure, so the host side can see the case each one settles as.
//! An order that does not decode is a `BadRequest`, a pricing call the
//! upstream did not answer is a `BadGateway`, and a refused ledger entry is a
//! `ServerError`.

#![cfg(target_arch = "wasm32")]

use omnia_sdk::api::messaging::{self, Delivery};
use omnia_sdk::api::{Context, Handlers, Router};
use omnia_sdk::{Error, WasiProvider};
use serde_json::Value;

messaging::export!(router);

fn router(provider: WasiProvider) -> Router<WasiProvider> {
    Handlers::new()
        .topic("orders", order)
        .topic_with("pricing", pricing, |delivery: &Delivery| Ok(delivery.clone()))
        .topic_with("ledger", ledger, |delivery: &Delivery| Ok(delivery.clone()))
        .with_state(provider)
}

// the suite's order is not JSON, so the router refuses it before this runs
async fn order(_: Context<WasiProvider>, _: Value) -> omnia_sdk::Result<()> {
    Ok(())
}

async fn pricing(_: Context<WasiProvider>, _: Delivery) -> omnia_sdk::Result<()> {
    Err(Error::bad_gateway("unavailable", "the pricing service did not answer"))
}

async fn ledger(_: Context<WasiProvider>, _: Delivery) -> omnia_sdk::Result<()> {
    Err(Error::server_error("other", "the ledger refused the entry"))
}
