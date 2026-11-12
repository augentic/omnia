# Document Store

The `omnia:docstore` interface is a JSON document store: named collections of `{ id, data }` documents with a rich filter language and cursor-based pagination. The default backend is in-memory; production deployments swap in Azure Table Storage (`omnia-azure-table`) without guest changes, for a guest whose queries keep to the [portable subset](#filters).

The [`docstore`](../../examples/docstore/) example is a minimal GTFS-flavoured service (a `stops` collection) showing CRUD and a combined query endpoint; the snippets below extend its patterns to the full filter vocabulary.

> **Host prerequisite.** The runtime serving your guest must link this interface: add `WasiDocStore: DocStoreDefault` (from `omnia_wasi_docstore`) to the `runtime!` `hosts:` map — see [Composing a Runtime](composing-a-runtime.md). Everything below is guest code.

## The provider

Guest access goes through the `DocumentDb` trait from `omnia-sdk` — call its methods on `WasiProvider` (or through a generic provider `P: DocumentDb`):

```rust,noplayground
use omnia_sdk::docstore::{Document, Exists, Filter, Query, ScalarValue, SortField};
use omnia_sdk::{DocumentDb, DocumentDbExt as _, WasiProvider, bad_request};
```

## CRUD

A `Document` is an id plus raw JSON bytes. `Document::json` serializes your own type into `data`, `Document::decode` reads it back, and `get_as` (from `DocumentDbExt`, blanket-implemented for every `DocumentDb`) fetches and decodes in one call. A body that is not the JSON of the type fails with `ErrorKind::Invalid`:

```rust,noplayground
async fn create_stop(Json(req): Json<CreateStopRequest>) -> omnia_sdk::Result<Json<Value>> {
    let doc = Document::json(req.id.clone(), &req.stop).context("serializing stop")?;
    if let Err(Exists) = WasiProvider.insert("stops", &doc).await.context("inserting stop")? {
        return Err(bad_request!("stop {} already exists", req.id));
    }
    Ok(Json(json!({ "stop": req.stop, "id": req.id })))
}

async fn get_stop(Path(id): Path<String>) -> omnia_sdk::Result<Json<Stop>> {
    let stop: Stop = WasiProvider
        .get_as("stops", &id)
        .await
        .context("reading stop")?
        .ok_or_else(|| not_found!("stop {id} not found"))?;
    Ok(Json(stop))
}
```

The four operations:

| Method | Semantics |
| ------ | --------- |
| `insert(collection, &doc)` | Create; answers `Ok(Err(Exists))` and leaves the stored document untouched if the id exists |
| `get(collection, id)` | Fetch by id → `Option<Document>` |
| `put(collection, &doc)` | Upsert (create or replace) |
| `delete(collection, id)` | Remove → `bool` (whether anything was deleted) |

`get_as::<T>(collection, id)` is `get` with the body decoded into `T`. Collections are created implicitly on first write. Every method fails with a `capability::Error`, which `?` converts into `omnia_sdk::Error`.

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

The backend translates the tree to its native query language (direct JSON evaluation in the default, OData `$filter` for Azure Table). Only the [portable subset](../reference/docstore.md#query-contract) runs on every backend:

- `eq`, `ne`, `gt`, `gte`, `lt`, and `lte` with a non-null value, on a top-level field;
- `in_list` and `not_in_list` on a top-level field;
- `and`, `or`, and `negate`;
- `limit`, with the `next` cursor.

The rest is optional: the string predicates, the null predicates, a comparison with `null`, a dotted field path, `order_by`, and `offset`. A backend serves each optional feature or fails it with `ErrorKind::Unsupported`, and never ignores one. A query that keeps to the portable subset runs alike locally and in production.

## Queries, sorting, and pagination

`query(collection, &Query)` combines the filter with sorting and cursor pagination:

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
let page = WasiProvider.query("stops", &query).await.context("querying stops")?;

let stops: Vec<Stop> = page.items.iter().map(Document::decode).collect::<Result<_, _>>()?;
Ok(Json(json!({ "stops": stops, "next": page.next })))
```

The result is a `capability::Page<Document>`: the page's `items`, plus an opaque `next` cursor when more pages exist. Hand the cursor back as the next query's `after` to resume. Treat it as opaque — its format is backend-specific.

## Backends

| Backend | Notes |
| ------- | ----- |
| `DocStoreDefault` (in-tree) | In-memory; state is process-local and lost on exit |
| `omnia-azure-table` | Azure Table Storage over REST; `AZURE_STORAGE_ACCOUNT`, `AZURE_STORAGE_KEY`, optional `AZURE_TABLE_ENDPOINT` (points at Azurite for local emulation) |

## Choosing docstore vs SQL vs keyvalue

- **keyvalue** — opaque bytes by key; no querying.
- **docstore** — schemaless JSON documents, filtered and paginated; no joins or transactions.
- **[SQL](sql.md)** — relational schema, joins, typed columns.
