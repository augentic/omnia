//! Transactions walked end to end over the raw `omnia:sql` bindings, against
//! the `SQLite` default, for what the SDK cannot reach.
//!
//! - `rollback` after `rollback` succeeds, and every call after `commit` is
//!   `invalid`.
//! - A transaction outlives the `database` handle it began on.
//! - A read on the database, joined with the transaction's `rollback`, waits
//!   for it and never sees the uncommitted row.
//! - `SAVEPOINT` is `invalid`, and ends the transaction.
//! - A `BEGIN` behind a byte-order mark passes the host's check, and the
//!   backend refuses it as `invalid`, leaving the database usable.

#![cfg(target_arch = "wasm32")]

use omnia_sdk::sql::database::Database;
use omnia_sdk::sql::types::{Error, ResultSet, Value};
use omnia_sdk::sql::{store, transactions};

omnia_sdk::command::export!(scenario);

const INSERT: &str = "INSERT INTO items (id, name) VALUES ($1, $2)";

async fn scenario() {
    let database = store::open("default".to_owned()).await.expect("open");
    let create = "CREATE TABLE items (id BIGINT PRIMARY KEY, name TEXT NOT NULL)";
    database.execute(create.to_owned(), vec![]).await.expect("create");

    // a second rollback succeeds
    let rolled_back = transactions::begin(&database).await.expect("begin");
    rolled_back.rollback().await.expect("rollback");
    rolled_back.rollback().await.expect("a second rollback");

    // every call after a commit is `invalid`
    let committed = transactions::begin(&database).await.expect("begin");
    let inserted = committed.execute(INSERT.to_owned(), item(1, "committed")).await;
    assert_eq!(inserted.expect("insert"), 1);
    committed.commit().await.expect("commit");
    let calls = [
        ("execute", committed.execute(INSERT.to_owned(), item(2, "late")).await.map(drop)),
        ("query", committed.query("SELECT 1".to_owned(), vec![]).await.map(drop)),
        ("commit", committed.commit().await),
        ("rollback", committed.rollback().await),
    ];
    for (call, result) in calls {
        assert!(matches!(result, Err(Error::Invalid(_))), "{call} after a commit: {result:?}");
    }

    // a transaction outlives the handle it began on
    let opened = store::open("default".to_owned()).await.expect("open");
    let outliving = transactions::begin(&opened).await.expect("begin");
    drop(opened);
    let inserted = outliving.execute(INSERT.to_owned(), item(3, "outlived")).await;
    assert_eq!(inserted.expect("insert"), 1);
    outliving.commit().await.expect("commit");

    // a read joined with the rollback waits for it
    let pending = transactions::begin(&database).await.expect("begin");
    let inserted = pending.execute(INSERT.to_owned(), item(4, "uncommitted")).await;
    assert_eq!(inserted.expect("insert"), 1);
    let (read, rollback) = futures::join!(names(&database), pending.rollback());
    rollback.expect("rollback");
    assert_eq!(read, ["committed", "outlived"], "a read saw an uncommitted row");

    // `SAVEPOINT` is refused, and ends the transaction
    let saving = transactions::begin(&database).await.expect("begin");
    let inserted = saving.execute(INSERT.to_owned(), item(5, "saving")).await;
    assert_eq!(inserted.expect("insert"), 1);
    let savepoint = saving.execute("SAVEPOINT inner".to_owned(), vec![]).await;
    assert!(matches!(savepoint, Err(Error::Invalid(_))), "{savepoint:?}");
    let after = saving.execute(INSERT.to_owned(), item(6, "after")).await;
    assert!(matches!(after, Err(Error::Invalid(_))), "{after:?}");
    drop(saving);

    // the backend refuses a `BEGIN` the host's check reads past
    let hidden = database.execute("\u{feff}BEGIN".to_owned(), vec![]).await;
    assert!(
        matches!(&hidden, Err(Error::Invalid(message)) if message.contains("opened a transaction")),
        "{hidden:?}"
    );
    let inserted = database.execute(INSERT.to_owned(), item(7, "usable")).await;
    assert_eq!(inserted.expect("insert"), 1);
    assert_eq!(names(&database).await, ["committed", "outlived", "usable"]);
}

fn item(id: i64, name: &str) -> Vec<Option<Value>> {
    vec![Some(Value::Int64(id)), Some(Value::Text(name.to_owned()))]
}

async fn names(database: &Database) -> Vec<String> {
    let selected = database.query("SELECT name FROM items ORDER BY id".to_owned(), vec![]).await;
    let ResultSet { rows, finished, .. } = selected.expect("select");
    let rows = rows.collect().await;
    finished.await.expect("finished");
    rows.into_iter()
        .map(|row| match &row[..] {
            [Some(Value::Text(name))] => name.clone(),
            other => panic!("read {other:?}"),
        })
        .collect()
}
