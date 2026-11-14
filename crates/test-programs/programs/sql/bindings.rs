//! Named databases through the `omnia_sdk` `SqlDb`: the unnamed handle and
//! `second` are separate databases, and `execute` counts the rows a
//! statement touched.

#![cfg(target_arch = "wasm32")]

use omnia_sdk::sql::Value;
use omnia_sdk::{SqlDb, WasiProvider};

omnia_sdk::api::command::export!(scenario);

async fn scenario() {
    let second = WasiProvider.named("second");

    // the same table in each database holds its own rows
    let create = "CREATE TABLE items (name TEXT)";
    assert_eq!(WasiProvider.execute(create, &[]).await.expect("create in default"), 0);
    assert_eq!(second.execute(create, &[]).await.expect("create in second"), 0);
    let insert = "INSERT INTO items (name) VALUES ($1), ($2)";
    let inserted = WasiProvider.execute(insert, &[Value::from("alpha"), Value::from("beta")]).await;
    assert_eq!(inserted.expect("insert into default"), 2);
    let insert = "INSERT INTO items (name) VALUES ($1)";
    let inserted = second.execute(insert, &[Value::from("gamma")]).await;
    assert_eq!(inserted.expect("insert into second"), 1);
    assert_eq!(names(&WasiProvider).await, ["alpha", "beta"]);
    assert_eq!(names(&WasiProvider.named("default")).await, ["alpha", "beta"]);
    assert_eq!(names(&second).await, ["gamma"]);
}

async fn names(db: &impl SqlDb) -> Vec<String> {
    let rows = db.query("SELECT name FROM items ORDER BY name", &[]).await.expect("select");
    rows.iter().map(|row| row.get("name").expect("a text column")).collect()
}
