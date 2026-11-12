# SQL

The `omnia:sql` interface gives guests parameterized SQL access to whatever database the host configures — SQLite in development (`SqlDefault`), PostgreSQL in production (`omnia-postgres`). `omnia-sdk` provides the `SqlDb` capability over it: one trait whose methods each run in one host call, so handlers execute SQL through a provider like any other capability. The [SQL reference](../reference/sql.md) defines the interface, the portable dialect, and how each value binds and reads back on each backend.

The [`sql`](../../examples/sql/) example is a minimal CRUD service over a single `agency` table; the snippets below follow its patterns.

> **Host prerequisite.** The runtime serving your guest must link this interface: add `WasiSql: SqlDefault` (from `omnia_wasi_sql`) to the `runtime!` `hosts:` map — see [Composing a Runtime](composing-a-runtime.md). Everything below is guest code.

## The `SqlDb` capability

`omnia_sdk::SqlDb` runs SQL through three methods:

- `query` returns the rows a statement selects.
- `execute` returns the number of rows it changed. DDL runs through it too.
- `batch` runs a list of `Statement`s in order, in one transaction, and returns each one's count. A failing statement rolls back the whole batch.

`query` and `execute` take the SQL text and its `$n` parameters as a slice of `Value`s, which convert from Rust scalars and `Option`s of them. A provider's own handle runs statements on the database `default`, and `named(name)` answers a handle on another database the deployment declares. On `wasm32`, `omnia_sdk::WasiProvider` implements `SqlDb` over the WASI bindings:

```rust,noplayground
use omnia_sdk::sql::Value;
use omnia_sdk::{SqlDb, WasiProvider};
```

Insert with bound parameters — a `None` binds a NULL:

```rust,noplayground
WasiProvider
    .execute(
        "INSERT INTO agency (agency_id, name, url, timezone, created_at) \
         VALUES ($1, $2, $3, $4, $5)",
        &[
            Value::from(agency.agency_id),
            Value::from(agency.name.as_str()),
            Value::from(agency.url.as_deref()),
            Value::from(agency.timezone.as_deref()),
            Value::from(agency.created_at.as_str()),
        ],
    )
    .await
    .context("inserting agency")?;
```

Statements are always parameterized (`$1`, `$2`, ...) — string interpolation into SQL is never necessary and never safe. Every backend binds `$n` to the `n`th parameter wherever it appears, so `UPDATE items SET name = $2 WHERE id = $1` runs alike on `SQLite` and Postgres.

Delete, checking the affected-row count for a not-found result:

```rust,noplayground
let rows_affected = WasiProvider
    .execute("DELETE FROM agency WHERE agency_id = $1", &[Value::from(id)])
    .await
    .context("deleting agency")?;

if rows_affected == 0 {
    return Err(anyhow!("agency not found").into());
}
```

A statement against another database goes through its named handle:

```rust,noplayground
let events = WasiProvider.named("eventstore");
events.execute("DELETE FROM event WHERE agency_id = $1", &[Value::from(id)]).await?;
```

The database name is what the backend resolves, and both backends refuse a name they do not declare. `SqlDefault` declares `default` plus the names in `SQL_DATABASES`, and `omnia-postgres` maps `default` to `POSTGRES_URL` and each name in `POSTGRES_POOLS` to `POSTGRES_URL__<NAME>`.

Every method fails with a `capability::Error`, which `?` converts into `omnia_sdk::Error`. Its kind is the one the backend chose. An undeclared database fails with `ErrorKind::NotFound`, and a statement that does not prepare or bind with `ErrorKind::Invalid`. The [reference](../reference/sql.md#errors) lists every kind.

### Mapping rows

`query` returns `Vec<Row>`; each `Row` carries `fields`, a list of `Field { name, value }` in column order. `Row::get` reads one column by name as a Rust type — an integer, float, boolean, string, or binary scalar, or `Option` of one for a nullable column — and fails with a `ColumnError` when the column is absent, holds `NULL` for a non-`Option` type, or holds another kind of `Value`:

```rust,noplayground
let name: String = row.get("name")?;
let url: Option<String> = row.get("url")?;
```

A record type derives `FromRow` to read every field from the column of the same name, and `query_as` (from `SqlDbExt`, blanket-implemented for every `SqlDb`) runs a query and decodes each row. A row that does not decode fails with `ErrorKind::Invalid`:

```rust,noplayground
use omnia_sdk::SqlDbExt as _;
use omnia_sdk::sql::FromRow;

#[derive(Debug, Clone, Serialize, FromRow)]
struct Agency {
    agency_id: i64,
    name: String,
    url: Option<String>,
    timezone: Option<String>,
    created_at: String,
}

let agencies: Vec<Agency> = WasiProvider
    .query_as(
        "SELECT agency_id, name, url, timezone, created_at FROM agency ORDER BY created_at DESC",
        &[],
    )
    .await
    .context("executing query")?;
```

Each integer case reads as any integer type its value fits, so a column Postgres returns as `Int32` and SQLite as `Int64` reads as `i32` from both. `Float` and `Double` read as `f64`, and `bool` also reads an integer `0` or `1`, which is how SQLite returns a `BOOLEAN`. The default backend reports NULL as `Str(None)` regardless of column type, which `Option<T>` reads as `None` like any other `NULL`. The [reference](../reference/sql.md#values-and-column-types) lists what each backend returns for each column type.

Because `SqlDb` is a trait, handlers written against `P: SqlDb` run unchanged natively against `omnia_test::guest::Provider`, which scripts the rows and counts each statement returns and records the database each ran against — see [Testing Omnia-Based Code](testing-omnia-code.md).

## The `wasi:sql` draft

The host also serves the `wasi:sql` draft over the same backend, for a guest that calls its raw bindings. Open a connection by database name, prepare a statement, and execute it:

```rust,noplayground
async fn ensure_schema() -> Result<()> {
    let pool = Connection::open(omnia_sdk::sql::DEFAULT.to_owned())
        .await
        .map_err(|e| anyhow!("opening connection: {}", e.trace()))?;

    let create_agency = "CREATE TABLE IF NOT EXISTS agency (
        agency_id INTEGER PRIMARY KEY,
        name TEXT NOT NULL,
        url TEXT,
        timezone TEXT,
        created_at TEXT NOT NULL
    )";

    let stmt = Statement::prepare(create_agency.to_string(), vec![])
        .await
        .map_err(|e| anyhow!("preparing agency table creation: {}", e.trace()))?;

    readwrite::exec(&pool, &stmt)
        .await
        .map_err(|e| anyhow!("creating agency table: {}", e.trace()))?;

    Ok(())
}
```

`SqlDb` covers everything the draft does, so new code has no need for it: DDL runs through `execute` too. The draft has no `batch`, and its `error` resource carries the message without a kind. The [reference](../reference/sql.md#the-wasisql-draft) lists every difference.

> Each request runs in a fresh guest instance, so anything like `ensure_schema` runs per request. Real deployments manage schema migrations host-side or out-of-band; the in-example DDL is a demo convenience.

## Backends

| Backend | Notes |
| ------- | ----- |
| `SqlDefault` (in-tree) | SQLite. `SQL_DATABASE` selects the file of `default`, a shared in-memory database when unset. `SQL_DATABASES` lists further names, each in `SQL_DATABASE__<NAME>` or a private in-memory database. |
| `omnia-postgres` | PostgreSQL via connection pool(s); `POSTGRES_URL`, `POSTGRES_POOL_SIZE`, named pools via `POSTGRES_POOLS` |

Guest code is identical against both; keep to parameterized statements, the [portable dialect](../reference/sql.md#the-portable-dialect), and the database names both declare, and the swap is configuration only.
