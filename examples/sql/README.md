# SQL Example

Demonstrates `wasi-sql` using the default (in-memory) implementation: raw
prepared statements for schema creation, then CRUD over a single `agency`
table with hand-written parameterized SQL executed through the `TableStore`
capability, mapping result `Row`s back into a Rust struct.

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

- **Prepared statements** — schema creation via `Statement::prepare` + `readwrite::exec`
- **`TableStore`** — a unit `Provider` implementing the capability, executing SQL with `query` / `exec`
- **Parameterized filters** — `$1`, `$2`, ... placeholders bound from `DataType` values
- **Row mapping** — reading `Row` fields by name into a `Serialize` struct

See the [SQL guide](../../docs/guides/sql.md) for raw `wasi:sql` usage,
the `TableStore` capability, and backend selection.
