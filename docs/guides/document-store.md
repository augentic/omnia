# Document Store

The `omnia:docstore` interface is a JSON document store: named collections of `{ id, data }` documents with a rich filter language and cursor-based pagination. The default backend is in-memory; production deployments swap in Azure Table Storage (`omnia-azure-table`) without guest changes, for a guest whose queries keep to the [portable query](#filters).

The [`docstore`](../../examples/docstore/) example is a minimal GTFS-flavoured service (a `stops` collection) showing CRUD and a combined query endpoint; the snippets below extend its patterns to the full filter vocabulary.

> **Host prerequisite.** The runtime serving your guest must link this interface: add `WasiDocStore: DocStoreDefault` (from `omnia_wasi_docstore`) to the `runtime!` `hosts:` map — see [Composing a Runtime](composing-a-runtime.md) — and declare the collections the guest uses in `DOCSTORE_COLLECTIONS`. Everything below is guest code.

## Opening a collection

`omnia_sdk::docstore` carries the interface's bindings and the helpers over them. `store::open(name)` opens a `Collection` the deployment declares:

```rust,noplayground
use omnia_sdk::docstore::collection::Collection;
use omnia_sdk::docstore::types::Document;
use omnia_sdk::docstore::{Exists, Filter, Query, ScalarValue, SortField, store};
use omnia_sdk::{bad_request, not_found};

async fn stops() -> omnia_sdk::Result<Collection> {
    Ok(store::open("stops".to_owned()).await?)
}
```

The deployment declares its collections, and a write never creates one. The default backend serves the names `DOCSTORE_COLLECTIONS` lists (comma-separated, default `default`), and opening any other name fails with the code `not-found`. The example runs as `DOCSTORE_COLLECTIONS=stops make example docstore`.

## CRUD

A `Document` is an id, its JSON body, and a revision. A collection's helpers store your own type as the body and read it back. A body that is not the JSON of the type fails with the code `invalid`:

```rust,noplayground
async fn create_stop(Json(req): Json<CreateStopRequest>) -> omnia_sdk::Result<Json<Value>> {
    if let Err(Exists) = stops().await?.insert(&req.id, &req.stop).await? {
        return Err(bad_request!("stop {} already exists", req.id));
    }
    Ok(Json(json!({ "stop": req.stop, "id": req.id })))
}

async fn get_stop(Path(id): Path<String>) -> omnia_sdk::Result<Json<Stop>> {
    let stop: Stop =
        stops().await?.get_as(&id).await?.ok_or_else(|| not_found!("stop {id} not found"))?;
    Ok(Json(stop))
}
```

The four operations:

| Method | Semantics |
| ------ | --------- |
| `insert(id, &value)` | Create; answers `Ok(Err(Exists))` and leaves the stored document untouched if the id exists |
| `get_as::<T>(id)` | Fetch by id, decoded → `Option<T>` |
| `upsert(id, &value)` | Create or replace |
| `delete(id, None)` | Remove → `bool` (whether anything was deleted) |

The generated `get(id)` answers the stored `Option<Document>`, and `Document::decode` decodes its body. A body is a JSON object, and any other body fails with the code `invalid`. `delete` with a revision removes the document only while it holds that revision, and answers a `Conflict` otherwise. Every call fails with the shared `types::Error`, which `?` converts into an `omnia_sdk::Error` under the code the [reference](../reference/docstore.md#errors) lists.

## Filters

Filters reference fields *inside* the document JSON and compose into trees. Available predicates:

| Filter | Meaning |
| ------ | ------- |
| `Filter::eq(field, value)` / `ne` | Equality / inequality |
| `Filter::gte(field, value)` / `lte` | Range bounds |
| `Filter::contains(field, text)` | Substring match |
| `Filter::in_list(field, values)` | Membership in a `Vec<ScalarValue>` |
| `Filter::is_null(field)` / `is_not_null(field)` | Presence checks |
| `Filter::on_date(field, "YYYY-MM-DD")` | Date-day match on a timestamp field (fallible — validates the date) |
| `Filter::and(filters)` / `Filter::or(filters)` | Boolean composition |
| `Filter::negate(filter)` | Logical NOT of any sub-tree |

A `Timestamp` value, such as the bounds `on_date` builds, compares as text. ISO-8601 instants written in one zone and format therefore order correctly.

Building a filter from optional query parameters is the common pattern — collect predicates, then `and` them:

```rust,noplayground
let mut filters = Vec::new();

if let Some(q) = &p.q {
    filters.push(Filter::contains("stop_name", q));
}
if let Some(zone) = &p.zone {
    filters.push(Filter::eq("zone_id", zone.as_str()));
}
if let Some(zone) = &p.exclude_zone {
    filters.push(Filter::ne("zone_id", zone.as_str()));
}
if p.accessible.unwrap_or(false) {
    filters.push(Filter::eq("wheelchair_boarding", 1));
    filters.push(Filter::is_not_null("zone_id"));
}
if p.top_level.unwrap_or(false) {
    filters.push(Filter::is_null("parent_station"));
}
if let Some(v) = p.min_lat {
    filters.push(Filter::gte("stop_lat", v));
}
if let Some(v) = p.max_lat {
    filters.push(Filter::lte("stop_lat", v));
}
if let Some(v) = p.min_lon {
    filters.push(Filter::gte("stop_lon", v));
}
if let Some(v) = p.max_lon {
    filters.push(Filter::lte("stop_lon", v));
}
if let Some(date) = &p.updated_on {
    filters.push(Filter::on_date("last_updated", date)?);
}

let filter = if filters.is_empty() { None } else { Some(Filter::and(filters)) };
```

Nested composition — OR across fields, and negated conjunctions:

```rust,noplayground
if let Some(q) = &p.q {
    filters.push(Filter::or([
        Filter::contains("route_short_name", q),
        Filter::contains("route_long_name", q),
    ]));
}
if let Some(types_str) = &p.types {
    let type_vals: Vec<ScalarValue> = types_str
        .split(',')
        .filter_map(|s| s.trim().parse::<i32>().ok())
        .map(ScalarValue::from)
        .collect();
    if !type_vals.is_empty() {
        filters.push(Filter::in_list("route_type", type_vals));
    }
}
if let Some(agency) = &p.agency {
    filters.push(Filter::eq("agency_id", agency.as_str()));
}
if let Some(exclude) = p.exclude_type {
    filters.push(Filter::negate(Filter::eq("route_type", exclude)));
}
if let (Some(agency), Some(rtype)) = (&p.not_agency, p.not_type) {
    filters.push(Filter::negate(Filter::and([
        Filter::eq("agency_id", agency.as_str()),
        Filter::eq("route_type", rtype),
    ])));
}
```

Before a query leaves the guest, the SDK writes the tree as an `and` of `or`s. It pushes each `negate` down to the fields and distributes each `or` over the `and`s inside it, so any nesting reaches every backend in the same form. A filter that holds more than `Filter::MAX_CONDITIONS` (100) conditions once written that way fails with the code `invalid`.

The backend translates the filter to its native query language (direct JSON evaluation in the default, OData `$filter` for Azure Table). Only the [portable query](../reference/docstore.md#query-contract) runs on every backend:

- `eq`, `ne`, `gt`, `gte`, `lt`, and `lte` with a non-null value, on a top-level field;
- `in_list` and `not_in_list` on a top-level field;
- `and`, `or`, and `negate`, nested in any way;
- at most 15 comparisons, counting each value of a list;
- no `order_by`;
- `limit`, with the `next` cursor.

The rest is optional: the string predicates, the null predicates, a comparison with `null`, a list that holds `null`, a dotted field path, and `order_by`. A backend serves each optional feature or fails it with the code `unsupported`, and never ignores one. A query that keeps to the portable query runs alike locally and in production.

## Queries, sorting, and pagination

`find(&Query)` combines the filter with sorting and cursor pagination:

```rust,noplayground
let query = Query {
    filter,
    order_by: vec![SortField {
        field: "stop_name".into(),
        descending: false,
    }],
    limit: p.limit,
    after: p.after,
    ..Default::default()
};
let page = stops().await?.find(&query).await?;

let stops: Vec<Stop> = page.documents.iter().map(Document::decode).collect::<Result<_, _>>()?;
Ok(Json(json!({ "stops": stops, "next": page.next })))
```

The result is a `collection::Page`: the page's `documents`, plus an opaque `next` cursor when more pages exist. Hand the cursor back as the next query's `after` to resume. Treat it as opaque — its format is backend-specific. A page holds at most 1,000 documents. The cursor is the only way to page, and a query that sets `offset` fails with the code `unsupported`.

## Backends

| Backend | Notes |
| ------- | ----- |
| `DocStoreDefault` (in-tree) | In-memory; serves the collections `DOCSTORE_COLLECTIONS` declares; state is process-local and lost on exit |
| `omnia-azure-table` | Azure Table Storage over REST; `AZURE_STORAGE_ACCOUNT`, `AZURE_STORAGE_KEY`, optional `AZURE_TABLE_ENDPOINT` (points at Azurite for local emulation) |

## Choosing docstore vs SQL vs keyvalue

- **keyvalue** — opaque bytes by key; no querying.
- **docstore** — schemaless JSON documents, filtered and paginated; no joins or transactions.
- **[SQL](sql.md)** — relational schema, joins, typed columns.
