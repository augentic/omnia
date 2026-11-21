//! The SDK's SQL helpers inside an axum handler, against the `SQLite`
//! default. axum's `Handler` bound requires a `Send` future, so the guest
//! build checks it for a handler that runs `Database::transaction`, and for a
//! fn over `&impl Executor` whatever the executor.
//!
//! - Work that returns `Ok` commits, and work that returns `Err` rolls back
//!   and fails with that error.
//! - A fn over `&impl Executor` runs in a transaction and on the database
//!   alike.
//! - `params!` binds each argument, and a tuple decodes its row by position.

#![cfg(target_arch = "wasm32")]

use axum::Json;
use axum::handler::Handler;
use omnia_sdk::sql::{self, Executor};
use omnia_sdk::{Result, bad_request};
use serde::Deserialize;

omnia_sdk::command::export!(scenario);

const QUOTA: i64 = 2;

#[derive(Deserialize)]
struct Item {
    id: i64,
    name: String,
}

async fn scenario() {
    fn assert_handler<H: Handler<T, ()>, T>(handler: H) -> H {
        handler
    }
    fn assert_send<E: Executor>(db: &E) -> impl Future<Output = Result<i64>> + Send + '_ {
        count(db)
    }

    let db = sql::open(sql::DEFAULT).await.expect("open");
    let create = "CREATE TABLE items (id BIGINT PRIMARY KEY, name TEXT NOT NULL)";
    db.exec(create, sql::params![]).await.expect("create");
    let place = assert_handler(place);

    let Json(placed) = place(item(1, "kept")).await.expect("the first item fits");
    assert_eq!(placed, [(1, "kept".to_owned())]);
    let Json(placed) = place(item(2, "second")).await.expect("the second item fits");
    assert_eq!(placed, [(1, "kept".to_owned()), (2, "second".to_owned())]);

    let err = place(item(3, "refused")).await.expect_err("the third item is over quota");
    assert_eq!(err.code(), "bad_request", "{err}");
    assert_eq!(assert_send(&db).await.expect("count"), QUOTA, "the refused insert rolled back");
}

async fn place(Json(item): Json<Item>) -> Result<Json<Vec<(i64, String)>>> {
    let db = sql::open(sql::DEFAULT).await?;
    let insert = "INSERT INTO items (id, name) VALUES ($1, $2)";
    db.transaction(async |tx| {
        tx.exec(insert, sql::params![item.id, item.name.as_str()]).await?;
        if count(tx).await? > QUOTA {
            return Err(bad_request!("over the quota of {QUOTA} items"));
        }
        Ok(())
    })
    .await?;
    Ok(Json(db.query_as("SELECT id, name FROM items ORDER BY id", sql::params![]).await?))
}

async fn count(db: &impl Executor) -> Result<i64> {
    let counted: Option<(i64,)> =
        db.query_opt("SELECT COUNT(*) FROM items", sql::params![]).await?;
    Ok(counted.map_or(0, |(count,)| count))
}

fn item(id: i64, name: &str) -> Json<Item> {
    Json(Item {
        id,
        name: name.to_owned(),
    })
}
