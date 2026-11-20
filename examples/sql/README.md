# SQL Example

Demonstrates `omnia:sql` using the default (in-memory) implementation:
schema creation on the `default` database, then CRUD over a single `agency`
table with hand-written parameterized SQL executed through the `Database`
helpers, mapping result `Row`s back into a Rust struct.

## Quick Start

```bash
make example sql
```

Or, more manually, for debugging:

```bash
# build the guest
cargo build --example sql-wasm --target wasm32-wasip3

# run the host
cargo run --example sql -- -v run ./target/wasm32-wasip3/debug/examples/sql_wasm.wasm
```

## Test

```bash
# create an agency (INSERT)
curl -X POST http://localhost:8080/agencies \
  -H 'Content-Type: application/json' \
  -d '{"agency_id":1,"name":"Ritchies Transport","url":"https://ritchies.co.nz","timezone":"Pacific/Auckland"}'

# list agencies, newest first (SELECT ... ORDER BY)
curl http://localhost:8080/agencies

# delete an agency (DELETE; errors when no row matched)
curl -X DELETE http://localhost:8080/agencies/1
```

## Features Demonstrated

- **Opening a database** — `sql::open` on the `default` database, then schema creation with `exec`
- **`Database` helpers** — executing SQL with `query_as` / `exec`
- **Parameterized filters** — `$1`, `$2`, ... placeholders bound from `Value`s
- **Row mapping** — decoding each `Row` into a `Serialize` struct with `FromRow`

See the [SQL guide](../../docs/guides/sql.md) for the `omnia:sql`
bindings, their helpers, and backend selection.
