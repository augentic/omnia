# DocStore Example

Demonstrates `wasi:docstore` using the default in-memory backend: CRUD on a
single `stops` collection plus one query endpoint with filters, sorting,
limits, and continuation-token pagination.

## Quick Start

```bash
make build docstore
make run docstore
```

Or, more manually, for debugging:

```bash
# build the guest
cargo build --example docstore-wasm --target wasm32-wasip2

# run the host
export RUST_LOG="info,opentelemetry_sdk=off,omnia_wasi_docstore=debug,omnia_wasi_http=debug"
cargo run --example docstore -- run ./target/wasm32-wasip2/debug/examples/docstore_wasm.wasm
```

## Test

```bash
# create stops
curl -s -X POST http://localhost:8080/stops \
  -H 'Content-Type: application/json' \
  -d '{"id":"stop-001","stop_name":"Britomart Transport Centre","stop_lat":-36.8442,"stop_lon":174.7676,"zone_id":"zone-1"}'

curl -s -X POST http://localhost:8080/stops \
  -H 'Content-Type: application/json' \
  -d '{"id":"stop-002","stop_name":"Newmarket Station","stop_lat":-36.8690,"stop_lon":174.7779,"zone_id":"zone-1"}'

curl -s -X POST http://localhost:8080/stops \
  -H 'Content-Type: application/json' \
  -d '{"id":"stop-003","stop_name":"Albany Station","stop_lat":-36.7275,"stop_lon":174.6986,"zone_id":"zone-3"}'

# get by id
curl -s http://localhost:8080/stops/stop-001

# update (upsert)
curl -s -X PUT http://localhost:8080/stops/stop-001 \
  -H 'Content-Type: application/json' \
  -d '{"stop_name":"Britomart","stop_lat":-36.8442,"stop_lon":174.7676,"zone_id":"zone-1"}'

# query: all stops, sorted by name
curl -s "http://localhost:8080/stops"

# query: text search -- contains on stop_name
curl -s "http://localhost:8080/stops?q=Station"

# query: by zone -- eq on zone_id
curl -s "http://localhost:8080/stops?zone=zone-1"

# query: latitude range -- gte + lte
curl -s "http://localhost:8080/stops?min_lat=-36.90&max_lat=-36.80"

# pagination: limit, then follow the returned continuation token
curl -s "http://localhost:8080/stops?limit=2"
# curl -s "http://localhost:8080/stops?limit=2&continuation=<token>"

# delete
curl -s -X DELETE http://localhost:8080/stops/stop-003
```

The query endpoint builds `Filter::and(...)` from whichever query params are
present and sorts by `stop_name`. See the [Document Store
guide](../../docs/guides/document-store.md) for the full filter vocabulary
(`ne`, `in_list`, `is_null`, `or`, `not`, `on_date`, ...) and the
[docstore reference](../../docs/reference/docstore.md) for the interface
contract.
