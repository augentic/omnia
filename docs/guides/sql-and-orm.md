# SQL and the Guest ORM

The `wasi:sql` interface gives guests parameterized SQL access to whatever database the host configures — SQLite in development (`SqlDefault`), PostgreSQL in production (`omnia-postgres`). On top of the raw interface, `omnia-guest` provides a small ORM: an `entity!` macro that maps structs to tables and typed builders for select/insert/update/delete.

The [`sql`](../../examples/sql/) example is a complete CRUD service (agencies and feeds, with a JOIN endpoint); everything below is drawn from it.

## Raw SQL

Open a connection by pool name, prepare a statement, and execute it. Use this level for DDL and anything the builders don't cover:

```rust
{{#include ../../examples/sql/guest.rs:376:395}}
```

Statements are always parameterized (`$1`, `$2`, ...) — string interpolation into SQL is never necessary and never safe.

The pool name (`"db"` here) is what the backend resolves: the SQLite default ignores it, while `omnia-postgres` maps names to configured pools (`POSTGRES_POOLS` + `POSTGRES_URL__<NAME>`).

> Each request runs in a fresh guest instance, so anything like `ensure_schema` runs per request. Real deployments manage schema migrations host-side or out-of-band; the in-example DDL is a demo convenience.

## Defining entities

The `entity!` macro maps a struct to a table and generates column metadata plus a `from_row` constructor:

```rust
{{#include ../../examples/sql/guest.rs:419:429}}
```

`Option<T>` fields map to nullable columns. The struct is otherwise a normal struct — derive whatever you need.

## Queries with the builders

Builders produce `{ sql, params }` pairs; a provider (any type implementing `TableStore`) executes them. `query` returns rows, `exec` returns the affected-row count.

```rust
struct Provider;
impl TableStore for Provider {}
```

Select with filtering, ordering, and limits:

```rust
let select = SelectBuilder::<Agency>::new()
    .r#where(Filter::eq("agency_id", id))
    .order_by_desc(None, "created_at")
    .limit(100)
    .build()?;

let rows = Provider.query("db".to_string(), select.sql, select.params).await?;
let agencies: Vec<Agency> = rows.iter().map(Agency::from_row).collect::<Result<_>>()?;
```

Insert from an entity value:

```rust
let query = InsertBuilder::<Agency>::from_entity(&agency).build()?;
Provider.exec("db".to_string(), query.sql, query.params).await?;
```

Update only the fields that changed, guarded by a filter:

```rust
{{#include ../../examples/sql/guest.rs:183:198}}
```

Delete, checking the affected-row count for a not-found result:

```rust
{{#include ../../examples/sql/guest.rs:356:368}}
```

## Joins

An entity can span a JOIN. Fields not listed in `columns` resolve against the main table; listed ones pull from the joined table under an alias:

```rust
{{#include ../../examples/sql/guest.rs:445:463}}
```

Selecting `FeedWithAgency` then works exactly like a single-table entity — `order_by_desc(Some("feed"), "created_at")` qualifies the table when the column name is ambiguous.

## Backends

| Backend | Notes |
| ------- | ----- |
| `SqlDefault` (in-tree) | SQLite; `SQL_DATABASE` selects the file, default is a shared in-memory database |
| `omnia-postgres` | PostgreSQL via connection pool(s); `POSTGRES_URL`, `POSTGRES_POOL_SIZE`, named pools via `POSTGRES_POOLS` |

Guest code is identical against both; keep to parameterized statements and portable SQL types and the swap is configuration only.
