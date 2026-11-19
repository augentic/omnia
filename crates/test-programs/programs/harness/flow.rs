//! One guest that reaches every scripted and recording backend of
//! `omnia_test::host`, through both triggers the harness drives.
//!
//! - `POST /items/{id}` reads the item's name, renames it in a transaction,
//!   prices it upstream, takes a token, and announces the rename on
//!   `items.renamed` and to every WebSocket client. It answers `201` with
//!   the old name, the price, and the token.
//! - A delivery on `items.refresh` announces the item id it carries on
//!   `items.refreshed`, keyed by that id, and an empty one is rejected.

#![cfg(target_arch = "wasm32")]

use axum::Router;
use axum::extract::{Json, Path};
use axum::http::{Request, StatusCode};
use axum::routing::post;
use bytes::Bytes;
use omnia_sdk::api::http;
use omnia_sdk::exports::messaging::incoming_handler::Guest;
use omnia_sdk::identity::credentials;
use omnia_sdk::messaging::producer;
use omnia_sdk::messaging::types::{Client, Delivery, Message, Settle};
use omnia_sdk::sql::{DEFAULT, Row, store, transactions};
use omnia_sdk::websocket::{broadcast, types as websocket};
use omnia_sdk::{Result, not_found};
use serde::{Deserialize, Serialize};

http::export!(router);

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
    let db = store::open(DEFAULT.to_owned()).await?;
    let rows = db.query_as::<Row>("SELECT name FROM items WHERE id = $1", &[id.into()]).await?;
    let Some(row) = rows.first() else {
        return Err(not_found!("no item {id}"));
    };
    let previous: String = row.get("name")?;
    let transaction = transactions::begin(&db).await?;
    let update = "UPDATE items SET name = $1 WHERE id = $2";
    transaction.exec(update, &[input.name.as_str().into(), id.into()]).await?;
    transaction.commit().await?;

    // price it upstream, under a token of its own
    let identity = credentials::open("default".to_owned()).await?;
    let token = identity.token(&["prices.read"]).await?.token;
    let request = Request::get(format!("https://prices.test/items/{id}"))
        .header("authorization", format!("Bearer {token}"))
        .body(Bytes::new())
        .expect("a valid request");
    let Price { price } = omnia_sdk::http::fetch_json(request).await?;

    // announce the rename
    let client = Client::connect("default".to_owned()).await?;
    let renamed = Message::new(input.name.clone()).with_key(id.to_string());
    producer::send(&client, "items.renamed".to_owned(), renamed).await?;
    let sockets = websocket::Client::connect("default".to_owned()).await?;
    broadcast::send(&sockets, websocket::Message::Text(input.name), Vec::new()).await?;

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
omnia_sdk::messaging::export!(Messaging);

impl Guest for Messaging {
    async fn handle(delivery: Delivery) -> Result<(), Settle> {
        let id = delivery.message.data;
        if id.is_empty() {
            return Err(Settle::Reject("an empty refresh".to_owned()));
        }
        let key = String::from_utf8_lossy(&id).into_owned();

        let client = Client::connect("default".to_owned()).await.expect("connect");
        let refreshed = Message::new(id).with_key(key);
        producer::send(&client, "items.refreshed".to_owned(), refreshed).await.expect("send");
        Ok(())
    }
}
