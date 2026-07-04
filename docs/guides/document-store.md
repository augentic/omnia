# Document Store

The `wasi:docstore` interface is a JSON document store: named collections of `{ id, data }` documents with a rich, backend-portable filter language and cursor-based pagination. The default backend is an embedded PoloDB file; production deployments swap in Azure Table Storage (`omnia-azure-table`) without guest changes.

The [`docstore`](../../examples/docstore/) example is a GTFS-flavoured service (stops, routes, stop-times) that exercises every filter type; the snippets below come from it.

## The provider

Guest access goes through the `DocumentStore` trait from `omnia-guest` — implement it on a unit struct and call its methods:

```rust
use omnia_guest::DocumentStore;
use omnia_guest::document_store::{Document, Filter, QueryOptions, ScalarValue, SortField};

struct Provider;
impl DocumentStore for Provider {}
```

## CRUD

A `Document` is an id plus raw JSON bytes — serialize your own types into `data`:

```86:93:examples/docstore/guest.rs
async fn create_stop(Json(req): Json<CreateStopRequest>) -> HttpResult<Json<Value>> {
    let doc = Document {
        id: req.id.clone(),
        data: serde_json::to_vec(&req.stop).context("serializing stop")?,
    };
    Provider.insert("stops", &doc).await.context("inserting stop")?;
    Ok(Json(json!({ "stop": req.stop, "id": req.id })))
}
```

The four operations:

| Method | Semantics |
| ------ | --------- |
| `insert(collection, &doc)` | Create; fails if the id exists |
| `get(collection, id)` | Fetch by id → `Option<Document>` |
| `put(collection, &doc)` | Upsert (create or replace) |
| `delete(collection, id)` | Remove → `bool` (whether anything was deleted) |

Collections are created implicitly on first write.

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

```131:165:examples/docstore/guest.rs
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

```244:271:examples/docstore/guest.rs
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

The backend translates the tree to its native query language (PoloDB queries, OData `$filter` for Azure Table), so a filter that works locally works in production.

## Queries, sorting, and pagination

`query(collection, QueryOptions)` combines the filter with sorting and cursor pagination:

```167:185:examples/docstore/guest.rs
    let result = Provider
        .query(
            "stops",
            QueryOptions {
                filter,
                order_by: vec![SortField {
                    field: "stop_name".into(),
                    descending: false,
                }],
                limit: p.limit,
                continuation: p.continuation,
                ..Default::default()
            },
        )
        .await
        .context("querying stops")?;

    let stops = deserialize_docs(&result.documents)?;
    Ok(Json(json!({ "stops": stops, "continuation": result.continuation })))
```

The result carries `documents` plus an opaque `continuation` token when more pages exist; hand the token back in the next query's `continuation` to resume. Treat it as opaque — its format is backend-specific.

## Backends

| Backend | Notes |
| ------- | ----- |
| `DocStoreDefault` (in-tree) | Embedded PoloDB file; `DOCSTORE_DATABASE` selects the path (default: temp dir) |
| `omnia-azure-table` | Azure Table Storage over REST; `AZURE_STORAGE_ACCOUNT`, `AZURE_STORAGE_KEY`, optional `AZURE_TABLE_ENDPOINT` (points at Azurite for local emulation) |

## Choosing docstore vs SQL vs keyvalue

- **keyvalue** — opaque bytes by key; no querying.
- **docstore** — schemaless JSON documents, filtered and paginated; no joins or transactions.
- **[SQL](sql-and-orm.md)** — relational schema, joins, typed columns.
