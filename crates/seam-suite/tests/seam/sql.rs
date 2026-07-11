//! `wasi:sql` seam: the guest creates schema and inserts a row via prepared
//! statements, and a probe on the shared `SQLite` backend proves the insert
//! reached the host store.

use anyhow::{Context as _, Result};
use omnia_testkit::http;
use omnia_wasi_sql::{DataType, WasiSqlCtx as _};

use crate::fixture::{self, unique};

#[test]
fn insert_then_select() -> Result<()> {
    fixture::RT.block_on(async {
        let fx = fixture::conformance().await?;
        let name = unique("agency");

        let response =
            http::post_json(&fx.runtime, "/sql/agencies", format!(r#"{{"name":"{name}"}}"#))
                .await?;
        assert!(
            response.status().is_success(),
            "guest completes the SQL round-trip: {:?}",
            response.body()
        );
        let body: serde_json::Value = serde_json::from_slice(response.body())?;
        assert_eq!(body["agency"]["name"], name.as_str(), "the guest echoes the created agency");

        // The insert must be visible on the shared backend connection.
        let connection = fx.sql.open("db".to_owned()).await.context("open probe connection")?;
        let rows = connection
            .query(format!("SELECT name FROM agency WHERE name = '{name}'"), Vec::new())
            .await
            .context("query agencies")?;
        assert_eq!(rows.len(), 1, "the agency row reached the host store");
        assert!(
            matches!(&rows[0].fields[0].value, DataType::Str(Some(stored)) if *stored == name),
            "the inserted name reached the host store: {:?}",
            rows[0].fields
        );

        Ok(())
    })
}
