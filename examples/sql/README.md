# SQL Example

Demonstrates `wasi-sql` using the default (in-memory) implementation: raw
prepared statements for schema creation, then one endpoint per guest ORM
builder — `SelectBuilder`, `InsertBuilder`, `UpdateBuilder`, `DeleteBuilder` —
plus an `entity!` JOIN mapping across a two-table agency/feed schema.

## Quick Start

```bash
make build sql
make run sql
```

Or, more manually, for debugging:

```bash
# build the guest
cargo build --example sql-wasm --target wasm32-wasip2

# run the host
export RUST_LOG="info,opentelemetry_sdk=off,omnia_wasi_sql=debug,omnia_wasi_http=debug,sql=debug"
cargo run --example sql -- run ./target/wasm32-wasip2/debug/examples/sql_wasm.wasm
```

## Test

```bash
# create an agency (InsertBuilder)
curl -X POST http://localhost:8080/agencies \
  -H 'Content-Type: application/json' \
  -d '{"agency_id":1,"name":"Ritchies Transport","url":"https://ritchies.co.nz","timezone":"Pacific/Auckland"}'

# list agencies (SelectBuilder)
curl http://localhost:8080/agencies

# update an agency (UpdateBuilder)
curl -X PATCH http://localhost:8080/agencies/1 \
  -H 'Content-Type: application/json' \
  -d '{"name":"Ritchies Transport Agency","timezone":"Pacific/Auckland"}'

# create a feed for the agency (InsertBuilder + existence check)
curl -X POST http://localhost:8080/agencies/1/feeds \
  -H 'Content-Type: application/json' \
  -d '{"feed_id":1,"description":"Bus routes and schedules"}'

# list all feeds with agency info (entity! JOIN)
curl http://localhost:8080/feeds

# delete a feed (DeleteBuilder)
curl -X DELETE http://localhost:8080/feeds/1
```

## Features Demonstrated

- **Prepared statements** — schema creation via `Statement::prepare` + `readwrite::exec`
- **ORM entities** — the `entity!` macro, including a JOIN entity with column aliasing
- **Query builders** — `SelectBuilder` (with `order_by_desc`, `limit`), `InsertBuilder`, `UpdateBuilder`, `DeleteBuilder`
- **Parameterized filters** — `Filter::eq` WHERE clauses (`$1`, `$2`, ... placeholders)

See the [SQL and ORM guide](../../docs/guides/sql-and-orm.md) for the full
builder and filter vocabulary.
