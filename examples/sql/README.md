# SQL Example

Demonstrates `wasi-sql` using the default (in-memory) implementation with a two-table schema (agency/feed) showcasing JOINs, foreign keys, and full CRUD operations.

## Quick Start

```bash
# build the guest
cargo build --example sql-wasm --target wasm32-wasip2

# run the host
export RUST_LOG="info,qwasr_wasi_sql=debug,qwasr_wasi_http=debug,sql=debug"
cargo run --example sql -- run ./target/wasm32-wasip2/debug/examples/sql_wasm.wasm
```

## API Endpoints

### Agencies

**Create an agency**

```bash
curl -X POST http://localhost:8080/agencies \
  -H 'Content-Type: application/json' \
  -d '{"name":"Ritchies Transport","url":"https://ritchies.co.nz","timezone":"Pacific/Auckland"}'
```

**List all agencies**

```bash
curl http://localhost:8080/agencies
```

**Get specific agency**

```bash
curl http://localhost:8080/agencies/1
```

**Update an agency**

```bash
curl -X PATCH http://localhost:8080/agencies/1 \
  -H 'Content-Type: application/json' \
  -d '{"name":"Ritchies Transport Agency"}'
```

### Feeds

**Create a feed for an agency**

```bash
curl -X POST http://localhost:8080/agencies/1/feeds \
  -H 'Content-Type: application/json' \
  -d '{"description":"Bus routes and schedules"}'
```

**List feeds for a specific agency**

```bash
curl http://localhost:8080/agencies/1/feeds
```

**List all feeds with agency info (demonstrates JOIN)**

```bash
curl http://localhost:8080/feeds
```

**Delete a feed**

```bash
curl -X DELETE http://localhost:8080/feeds/1
```

## Features Demonstrated

- **ORM Entity Definition** - `entity!` macro with column aliasing
- **JOINs** - `FeedWithAgency` entity automatically joins agency table
- **Column Aliasing** - Selecting columns from joined tables
- **Foreign Keys** - Feed references agency via `agency_id`
- **CRUD Operations** - Full create, read, update, delete support
- **Filters** - WHERE clauses with type-safe filters
- **Query Building** - SelectBuilder, InsertBuilder, UpdateBuilder, DeleteBuilder
