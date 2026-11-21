# SQL

The `omnia:sql` interface gives guests parameterized SQL access to whatever database the host configures — SQLite in development (`SqlDefault`), PostgreSQL in production (`omnia-postgres`). `omnia-sdk`'s `sql` module carries its bindings, with helpers that bind `Value` parameters and decode rows. The [SQL reference](../reference/sql.md) defines the interface, the portable dialect, and how each value binds and reads back on each backend.

The [`sql`](../../examples/sql/) example is a minimal CRUD service over a single `agency` table; the snippets below follow its patterns.

> **Host prerequisite.** The runtime serving your guest must link this interface: add `WasiSql: SqlDefault` (from `omnia_wasi_sql`) to the `runtime!` `hosts:` map — see [Composing a Runtime](composing-a-runtime.md). Everything below is guest code.

## Opening a database

`sql::open(name)` opens a `Database` on a name the deployment declares, and `sql::default()` opens `sql::DEFAULT`. A `Database` runs SQL through four methods:

- `query_as` decodes the rows a statement selects (see [Mapping rows](#mapping-rows)).
- `query_opt` decodes the first row a statement selects, or answers `None` when it selects none.
- `exec` returns the number of rows a statement changed. DDL runs through it too, and returns zero.
- `batch` runs a list of `Statement`s in order, in one transaction, and returns each one's count. A failing statement rolls back the whole batch.

Each call commits before it returns, and `Database::transaction` runs work that spans several calls in one transaction (see [Transactions](#transactions)). `query_as`, `query_opt`, and `exec` take the SQL text and its `$n` parameters as a slice of `Value`s, which convert from Rust scalars and `Option`s of them. `sql::params![..]` lists the parameters as that slice, converting each. `Statement::new(sql, params)` takes the same pair.

```rust,noplayground
use omnia_sdk::sql;

let database = sql::default().await?;
```

Insert with bound parameters — a `None` binds a NULL:

```rust,noplayground
database
    .exec(
        "INSERT INTO agency (agency_id, name, url, timezone, created_at) \
         VALUES ($1, $2, $3, $4, $5)",
        sql::params![
            agency.agency_id,
            agency.name.as_str(),
            agency.url.as_deref(),
            agency.timezone.as_deref(),
            agency.created_at.as_str(),
        ],
    )
    .await
    .context("inserting agency")?;
```

Statements are always parameterized (`$1`, `$2`, ...) — string interpolation into SQL is never necessary and never safe. Every backend binds `$n` to the `n`th parameter wherever it appears, so `UPDATE items SET name = $2 WHERE id = $1` runs alike on `SQLite` and Postgres.

Delete, checking the affected-row count for a not-found result:

```rust,noplayground
let rows_affected = database
    .exec("DELETE FROM agency WHERE agency_id = $1", sql::params![id])
    .await
    .context("deleting agency")?;

if rows_affected == 0 {
    return Err(anyhow!("agency not found").into());
}
```

A statement against another database goes through a database opened on its name:

```rust,noplayground
let events = sql::open("eventstore").await?;
events.exec("DELETE FROM event WHERE agency_id = $1", sql::params![id]).await?;
```

The database name is what the backend resolves, and both backends refuse a name they do not declare. `SqlDefault` declares `default` plus the names in `SQL_DATABASES`, and `omnia-postgres` maps `default` to `POSTGRES_URL` and each name in `POSTGRES_POOLS` to `POSTGRES_URL__<NAME>`.

Every call fails with the shared `types::Error`, whose case is the code the backend chose, and `?` converts it into an `omnia_sdk::Error` under that code. Opening an undeclared database fails with the code `not-found`. A statement that does not prepare or bind fails with the code `invalid`, and so does one that starts or ends a transaction, such as `BEGIN` or `COMMIT`. The [reference](../reference/sql.md#errors) lists every code.

### Mapping rows

A record type derives `FromRow` to read every field from the column of the same name, and `query_as` runs a query and decodes each row. A row that does not decode fails with the code `invalid`:

```rust,noplayground
use omnia_sdk::sql::FromRow;

#[derive(Debug, Clone, Serialize, FromRow)]
struct Agency {
    agency_id: i64,
    name: String,
    url: Option<String>,
    timezone: Option<String>,
    created_at: String,
}

let agencies: Vec<Agency> = database
    .query_as(
        "SELECT agency_id, name, url, timezone, created_at FROM agency ORDER BY created_at DESC",
        &[],
    )
    .await
    .context("executing query")?;
```

A tuple of up to twelve column types is a `FromRow` too, decoding a row by position, so a query that selects a few columns needs no record type:

```rust,noplayground
let names: Vec<(i64, String)> =
    database.query_as("SELECT agency_id, name FROM agency", sql::params![]).await?;
```

`Row` is a `FromRow` as well, so `query_as::<Row>` answers the rows themselves. Each `Row` carries `fields`, a list of `Field { name, value }` in column order. `Row::get` reads one column by name as a Rust type — an integer, float, boolean, string, or binary scalar, or `Option` of one for a nullable column — and fails with a `ServerError` with the code `invalid` when the column is absent, holds `NULL` for a non-`Option` type, or holds another kind of `Value`:

```rust,noplayground
let name: String = row.get("name")?;
let url: Option<String> = row.get("url")?;
```

Each integer case reads as any integer type its value fits, so a column Postgres returns as `Int32` and SQLite as `Int64` reads as `i32` from both. `Float32` and `Float64` read as `f64`, and `bool` also reads an integer `0` or `1`, which is how SQLite returns a `BOOLEAN`. A date, time, or timestamp reads as a `String` in its case's format, and SQLite returns it exactly as bound. Every backend reads a `NULL` back as `Value::Null`, whatever its column's type, and `Option<T>` reads it as `None`. The [reference](../reference/sql.md#values-and-column-types) lists what each backend returns for each column type.

A handler test serves the database from `omnia_test::host::ScriptedTables`. It scripts the rows and counts each statement returns, and records the database each ran against and each transaction's statements and outcome — see [Testing Omnia-Based Code](testing-omnia-code.md#handler-tests-the-component-over-scripted-backends).

## Transactions

`database.transaction(work)` runs `work` over a `Transaction`, whose `query_as`, `query_opt`, and `exec` work as on the database. It commits when `work` returns `Ok`, rolls back when `work` returns `Err`, and answers what `work` returned:

```rust,noplayground
let debit = "UPDATE account SET balance = balance - $1 WHERE id = $2";
let credit = "UPDATE account SET balance = balance + $1 WHERE id = $2";
database
    .transaction(async |tx| {
        tx.exec(debit, sql::params![amount, from]).await?;
        tx.exec(credit, sql::params![amount, to]).await?;
        Ok(())
    })
    .await?;
```

`database.begin()` starts a `Transaction` that `commit` or `rollback` ends by hand. A fn over `&impl Executor` runs its statements on a `Database` or inside a `Transaction`, wherever its caller does. `Executor` is `Sync`, so such a fn's future is `Send` and an axum handler can await it.

- Dropping a transaction rolls it back, so an early return through `?` discards its changes.
- Any failure rolls it back too. After an `Err`, the transaction is over, and `commit` fails with the code `invalid`.
- A commit that fails committed nothing, unless its code is `unavailable`, when the outcome is unknown.
- A transaction holds one of the backend's connections until it ends, and a call on the database outside it may wait until then. So a handler awaits no other call on the database while a transaction is open. On the default backend, such a call fails with the code `unavailable` after five seconds.
- `batch` runs a fixed list of statements in one transaction, in one call. Use `transaction` when a later statement depends on what an earlier one read.
- There are no savepoints. `SAVEPOINT`, like `BEGIN` and `COMMIT`, fails with the code `invalid`. `INSERT ... ON CONFLICT` covers the usual reason for one, an insert that may collide with an existing row.

A `SET TRANSACTION ISOLATION LEVEL ...` statement, as a transaction's first, sets its isolation level on a backend that serves it, such as Postgres.

## Raw queries

The generated `query` and `execute` take the SQL text as a `String` and the parameters as `Option<types::Value>`s, where `None` binds a NULL. `query` returns a `ResultSet`:

- `columns` names each column in select order, even when the query selects no row.
- `rows` streams the rows, which the guest reads to its end before it awaits `finished`.
- `ResultSet::read` does both and answers the rows as `Row`s.

Most code needs only `query_as` and `exec`. The [reference](../reference/sql.md#interfaces) describes every function.

> Each request runs in a fresh guest instance, so schema DDL in a guest, as in the `sql` example, runs per request. Real deployments manage schema migrations host-side or out-of-band; the in-example DDL is a demo convenience.

## Backends

| Backend | Notes |
| ------- | ----- |
| `SqlDefault` (in-tree) | SQLite. `SQL_DATABASE` selects the file of `default`, a shared in-memory database when unset. `SQL_DATABASES` lists further names, each in `SQL_DATABASE__<NAME>` or a private in-memory database. |
| `omnia-postgres` | PostgreSQL via connection pool(s); `POSTGRES_URL`, `POSTGRES_POOL_SIZE`, named pools via `POSTGRES_POOLS` |

Guest code is identical against both; keep to parameterized statements, the [portable dialect](../reference/sql.md#the-portable-dialect), and the database names both declare, and the swap is configuration only.

The default backend is SQLite, so a test over it proves only what the portable dialect does. SQL outside the dialect, such as a Postgres-only function, is proven only against the production database.
