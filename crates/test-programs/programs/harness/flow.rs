//! One guest that reaches every scripted and recording backend of
//! `omnia_test::host`, through both triggers the harness drives, over the
//! default handles the SDK's modules open.
//!
//! - `POST /items/{id}` reads the item's name, renames it in a transaction,
//!   prices it upstream under a token, caches the price, drops the cached
//!   name, and announces the rename on `items.renamed` and to every
//!   WebSocket client. It answers `201` with the old name, the price, and
//!   the token.
//! - A delivery on `items.refresh` announces the item id it carries on
//!   `items.refreshed`, keyed by that id, and an empty one is rejected.

#![cfg(target_arch = "wasm32")]

use std::time::Duration;

use axum::Router;
use axum::extract::{Json, Path};
use axum::http::StatusCode;
use axum::routing::post;
use omnia_sdk::exports::messaging::incoming_handler::Guest;
use omnia_sdk::messaging::Message;
use omnia_sdk::messaging::types::{Delivery, Settle};
use omnia_sdk::{Result, http, identity, keyvalue, messaging, not_found, sql, websocket};
use serde::{Deserialize, Serialize};

http::export!(router);

// outlives the row, so the host still holds the price when it looks
const PRICE_TTL: Duration = Duration::from_mins(1);

fn router() -> Router {
    Router::new().route("/items/{id}", post(rename))
}

#[derive(Deserialize)]
struct Rename {
    name: String,
}

#[derive(Deserialize)]
struct Price {
    price: u64,
}

#[derive(Serialize)]
struct Renamed {
    previous: String,
    price: u64,
    token: String,
}

async fn rename(
    Path(id): Path<i64>, Json(input): Json<Rename>,
) -> Result<(StatusCode, Json<Renamed>)> {
    // read the item, then rename it
    let db = sql::default().await?;
    let select = "SELECT name FROM items WHERE id = $1";
    let Some((previous,)) = db.query_opt::<(String,)>(select, sql::params![id]).await? else {
        return Err(not_found!("no item {id}"));
    };
    let update = "UPDATE items SET name = $1 WHERE id = $2";
    db.transaction(async |tx| tx.exec(update, sql::params![input.name.as_str(), id]).await).await?;

    // price it upstream, under a token of its own
    let credentials = identity::default().await?;
    let token = credentials.token(&["prices.read"]).await?.token;
    let url = format!("https://prices.test/items/{id}");
    let Price { price } = http::get(url).bearer(&token).fetch_json().await?;

    // cache the price, and drop the name cached before the rename
    let cache = keyvalue::default().await?;
    let priced = price.to_string();
    cache.write(&format!("prices.{id}"), priced.as_bytes(), Some(PRICE_TTL)).await?;
    cache.remove(&format!("names.{id}")).await?;

    // announce the rename
    let client = messaging::default().await?;
    let renamed = Message::new(input.name.clone()).with_key(id.to_string());
    client.send("items.renamed", renamed).await?;
    let sockets = websocket::default().await?;
    sockets.broadcast(websocket::Message::Text(input.name), &[]).await?;

    Ok((
        StatusCode::CREATED,
        Json(Renamed {
            previous,
            price,
            token,
        }),
    ))
}

struct Messaging;
omnia_sdk::exports::messaging::export!(Messaging);

impl Guest for Messaging {
    async fn handle(delivery: Delivery) -> Result<(), Settle> {
        let id = delivery.message.data;
        if id.is_empty() {
            return Err(Settle::Reject("an empty refresh".to_owned()));
        }
        let key = String::from_utf8_lossy(&id).into_owned();

        let client = messaging::default().await.expect("connect");
        let refreshed = Message::new(id).with_key(key);
        client.send("items.refreshed", refreshed).await.expect("send");
        Ok(())
    }
}
