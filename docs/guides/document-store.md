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

```rust
{{#include ../../examples/docstore/guest.rs:86:93}}
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

```rust
{{#include ../../examples/docstore/guest.rs:131:165}}
```

Nested composition — OR across fields, and negated conjunctions:

```rust
{{#include ../../examples/docstore/guest.rs:244:271}}
```

The backend translates the tree to its native query language (PoloDB queries, OData `$filter` for Azure Table), so a filter that works locally works in production.

## Queries, sorting, and pagination

`query(collection, QueryOptions)` combines the filter with sorting and cursor pagination:

```rust
{{#include ../../examples/docstore/guest.rs:167:185}}
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
