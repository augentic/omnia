# SQL

The `wasi:sql` interface gives guests parameterized SQL access to whatever database the host configures — SQLite in development (`SqlDefault`), PostgreSQL in production (`omnia-postgres`). On top of the raw interface, `omnia-sdk` provides the `TableStore` capability (feature `sql`, on by default): one trait that opens the connection, prepares the statement, and runs it, so handlers execute SQL through a provider like any other capability.

The [`sql`](../../examples/sql/) example is a minimal CRUD service over a single `agency` table; the snippets below follow its patterns.

> **Host prerequisite.** The runtime serving your guest must link this interface: add `WasiSql: SqlDefault` (from `omnia_wasi_sql`) to the `runtime!` `hosts:` map — see [Composing a Runtime](composing-a-runtime.md). Everything below is guest code.

## Raw SQL

Open a connection by pool name, prepare a statement, and execute it. Use this level for DDL and anything that does not fit the capability's `query` / `exec` shape:

```rust,noplayground
async fn ensure_schema() -> Result<()> {
    let pool = Connection::open("db".to_string())
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

Statements are always parameterized (`$1`, `$2`, ...) — string interpolation into SQL is never necessary and never safe.

The pool name (`"db"` here) is what the backend resolves: the SQLite default ignores it, while `omnia-postgres` maps names to configured pools (`POSTGRES_POOLS` + `POSTGRES_URL__<NAME>`).

> Each request runs in a fresh guest instance, so anything like `ensure_schema` runs per request. Real deployments manage schema migrations host-side or out-of-band; the in-example DDL is a demo convenience.

## The `TableStore` capability

`omnia_sdk::TableStore` wraps the open/prepare/execute sequence behind two methods: `query` returns rows, `exec` returns the affected-row count. Both take the pool name, the SQL text, and the `$n` parameters as `DataType` values. On `wasm32` the default method bodies call the WASI bindings, so a guest provider is a unit struct with an empty impl:

```rust
struct Provider;
impl TableStore for Provider {}
```

Insert with bound parameters — `DataType::Str(None)` binds a NULL:

```rust,noplayground
Provider
    .exec(
        "db".to_string(),
        "INSERT INTO agency (agency_id, name, url, timezone, created_at) \
         VALUES ($1, $2, $3, $4, $5)"
            .to_string(),
        vec![
            DataType::Int64(Some(agency.agency_id)),
            DataType::Str(Some(agency.name.clone())),
            DataType::Str(agency.url.clone()),
            DataType::Str(agency.timezone.clone()),
            DataType::Str(Some(agency.created_at.clone())),
        ],
    )
    .await
    .context("inserting agency")?;
```

Delete, checking the affected-row count for a not-found result:

```rust,noplayground
let rows_affected = Provider
    .exec(
        "db".to_string(),
        "DELETE FROM agency WHERE agency_id = $1".to_string(),
        vec![DataType::Int64(Some(id))],
    )
    .await
    .context("deleting agency")?;

if rows_affected == 0 {
    return Err(anyhow!("agency not found").into());
}
```

### Mapping rows

`query` returns `Vec<Row>`; each `Row` carries `fields`, a list of `Field { name, value }` in column order. Look columns up by name and match on the `DataType` variant:

```rust,noplayground
let rows = Provider
    .query(
        "db".to_string(),
        "SELECT agency_id, name, url, timezone, created_at FROM agency ORDER BY created_at DESC"
            .to_string(),
        vec![],
    )
    .await
    .context("executing query")?;

let agencies = rows.iter().map(Agency::from_row).collect::<Result<Vec<_>>>()?;
```

```rust,noplayground
impl Agency {
    fn from_row(row: &Row) -> Result<Self> {
        Ok(Self {
            agency_id: int(field(row, "agency_id")?)?,
            name: text(field(row, "name")?)?,
            url: nullable_text(field(row, "url")?)?,
            timezone: nullable_text(field(row, "timezone")?)?,
            created_at: text(field(row, "created_at")?)?,
        })
    }
}

fn field<'a>(row: &'a Row, name: &str) -> Result<&'a DataType> {
    row.fields
        .iter()
        .find(|f| f.name == name)
        .map(|f| &f.value)
        .ok_or_else(|| anyhow!("missing column `{name}`"))
}

fn int(value: &DataType) -> Result<i64> {
    match value {
        DataType::Int64(Some(v)) => Ok(*v),
        DataType::Int32(Some(v)) => Ok(i64::from(*v)),
        other => bail!("expected integer, got {other:?}"),
    }
}

fn text(value: &DataType) -> Result<String> {
    nullable_text(value)?.ok_or_else(|| anyhow!("unexpected NULL"))
}

// The default backend reports NULL as `Str(None)` regardless of column type.
fn nullable_text(value: &DataType) -> Result<Option<String>> {
    match value {
        DataType::Str(v) => Ok(v.clone()),
        other => bail!("expected text, got {other:?}"),
    }
}
```

Because `TableStore` is a trait, handlers written against `P: TableStore` run unchanged natively against `omnia_test::guest::Provider`, which scripts the rows and counts each statement returns — see [Testing Omnia-Based Code](testing-omnia-code.md).

## Backends

| Backend | Notes |
| ------- | ----- |
| `SqlDefault` (in-tree) | SQLite; `SQL_DATABASE` selects the file, default is a shared in-memory database |
| `omnia-postgres` | PostgreSQL via connection pool(s); `POSTGRES_URL`, `POSTGRES_POOL_SIZE`, named pools via `POSTGRES_POOLS` |

Guest code is identical against both; keep to parameterized statements and portable SQL types and the swap is configuration only.
