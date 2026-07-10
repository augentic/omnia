//! Seam test for `wasi:sql`: drive the `sql` example guest through the real
//! WIT boundary and confirm the write landed in the shared `SQLite` backend.
//!
//! The guest creates its schema and inserts an agency via prepared statements,
//! so a `200` proves open/prepare/exec/query crossed the boundary, and querying
//! the shared backend afterwards proves the insert reached the host store.
//!
//! The guest is built automatically on first [`find_guest`] call; the test
//! skips locally when it is absent and fails under CI so the pipeline never
//! passes vacuously.

#![cfg(not(target_arch = "wasm32"))]

use anyhow::{Context as _, Result};
use omnia::wasmtime_wasi::ResourceTable;
use omnia::{Backend as _, HasHttp, Runtime};
use omnia_testkit::{http, single_guest};
use omnia_wasi_http::{HttpDefault, WasiHttp, WasiHttpCtxView};
use omnia_wasi_otel::{HasOtel, OtelDefault, WasiOtel, WasiOtelCtx};
use omnia_wasi_sql::{DataType, HasSql, SqlDefault, WasiSql, WasiSqlCtx};

/// The `examples/sql` backend bundle: `wasi:http` + `wasi:otel` + `wasi:sql`.
#[derive(Clone)]
struct Bundle {
    http: HttpDefault,
    otel: OtelDefault,
    sql: SqlDefault,
}

impl HasHttp for Bundle {
    fn http_view<'a>(&'a mut self, table: &'a mut ResourceTable) -> WasiHttpCtxView<'a> {
        self.http.as_view(table)
    }
}

impl HasOtel for Bundle {
    fn otel_ctx(&mut self) -> &mut dyn WasiOtelCtx {
        &mut self.otel
    }
}

impl HasSql for Bundle {
    fn sql_ctx(&mut self) -> &mut dyn WasiSqlCtx {
        &mut self.sql
    }
}

/// Build a single-guest runtime over `sql_wasm.wasm`, returning the runtime
/// and a handle to the shared `SQLite` backend (clones share one connection,
/// so this handle observes the guest's writes).
async fn runtime() -> Result<Option<(Runtime<Bundle>, SqlDefault)>> {
    let bundle = Bundle {
        http: HttpDefault::connect().await.context("connecting http")?,
        otel: OtelDefault::connect().await.context("connecting otel")?,
        sql: SqlDefault::connect().await.context("connecting sql")?,
    };
    let store_probe = bundle.sql.clone();

    let Some(guest) = single_guest("sql_wasm.wasm", bundle).await? else {
        return Ok(None);
    };
    let runtime =
        guest.host::<WasiHttp>()?.host::<WasiOtel>()?.host::<WasiSql>()?.into_runtime()?;
    Ok(Some((runtime, store_probe)))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn insert_then_select() -> Result<()> {
    let Some((runtime, sql)) = runtime().await? else {
        return Ok(());
    };

    let response = http::post_json(
        &runtime,
        "/agencies",
        r#"{"name":"Metro Transit","url":"https://metro.example","timezone":"UTC"}"#,
    )
    .await?;
    assert!(
        response.status().is_success(),
        "guest completes the SQL round-trip: {:?}",
        response.body()
    );
    let body: serde_json::Value = serde_json::from_slice(response.body())?;
    assert_eq!(body["agency"]["name"], "Metro Transit", "the guest echoes the created agency");

    // The insert must be visible on the shared backend connection.
    let connection = sql.open("db".to_owned()).await.context("open probe connection")?;
    let rows = connection
        .query("SELECT name FROM agency".to_owned(), Vec::new())
        .await
        .context("query agencies")?;
    assert_eq!(rows.len(), 1, "one agency row reached the host store");
    assert!(
        matches!(&rows[0].fields[0].value, DataType::Str(Some(name)) if name == "Metro Transit"),
        "the inserted name reached the host store: {:?}",
        rows[0].fields
    );

    Ok(())
}
