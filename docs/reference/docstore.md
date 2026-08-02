# DocStore Interface Reference

Reference for the `wasi:docstore` interface (version `0.1.0`): the operations, the guest SDK types, the filter language and its host-enforced limits, and the backend matrix. The conceptual walk-through is in [Document Store](../guides/document-store.md); the authoritative WIT is [`crates/wasi-docstore/wit/docstore.wit`](../../crates/wasi-docstore/wit/docstore.wit).

- [Operations](#operations)
- [Types](#types)
- [Filters](#filters)
- [Sorting and pagination](#sorting-and-pagination)
- [Errors](#errors)
- [Backends implementing this interface](#backends-implementing-this-interface)

## Operations

```wit
get: async func(collection: string, id: string) -> result<option<document>, error>;
insert: async func(collection: string, doc: document) -> result<_, error>;
put: async func(collection: string, doc: document) -> result<_, error>;
delete: async func(collection: string, id: string) -> result<bool, error>;
query: async func(collection: string, options: query-options) -> result<query-result, error>;
```

| Operation | Semantics |
| --------- | --------- |
| `get` | Point read by id → `option<document>` (`none` when absent). |
| `insert` | Create; **fails if the id already exists**. |
| `put` | Unconditional upsert (create or replace). |
| `delete` | Remove by id → `bool` (whether anything was deleted). |
| `query` | Filtered read with sorting and pagination (see below). |

Collections are named by string and created implicitly on first write; there is no open/close handshake and no collection resource.

Guests normally call these through the `omnia_guest::DocumentStore` trait (implement it on a unit struct; the `wasm32` default methods delegate to `omnia_wasi_docstore::store`). The underlying guest functions are `omnia_wasi_docstore::store::{get, insert, put, delete, query}`.

## Types

### `document`

| Field | Type | Meaning |
| ----- | ---- | ------- |
| `id` | `string` | Primary key within the collection. |
| `data` | `list<u8>` | JSON-serialized document body. The guest serializes (`serde_json`); the backend stores or translates as needed. |

### `query-options`

| Field | Type | Meaning |
| ----- | ---- | ------- |
| `filter` | `option<filter>` | Filter tree; `none` matches everything. |
| `order-by` | `list<sort-field>` | Sort keys, first key wins, then the next. |
| `limit` | `option<u32>` | Maximum documents to return in this page. |
| `offset` | `option<u32>` | Skip this many documents after filter/sort. |
| `continuation` | `option<string>` | Opaque token from a previous `query-result`; `none` starts from the beginning. |

### `sort-field`

`field` (path into the document JSON) plus `descending` (`bool`, default ascending).

### `query-result`

`documents` (the page) plus `continuation` (`option<string>` — present when more pages exist; pass it back unchanged).

## Filters

Filters reference fields *inside* the document JSON and compose into trees. On the guest side, build them with the `Filter` constructors from `omnia_guest::document_store` (defined in [`crates/wasi-docstore/src/document_store.rs`](../../crates/wasi-docstore/src/document_store.rs)):

| Constructor | Predicate |
| ----------- | --------- |
| `Filter::eq(field, value)` / `ne` / `gt` / `gte` / `lt` / `lte` | Comparison against a scalar (`Filter::cmp` takes an explicit `ComparisonOp`). |
| `Filter::in_list(field, values)` / `not_in_list` | Set membership. |
| `Filter::is_null(field)` / `is_not_null(field)` | Null/missing checks. |
| `Filter::contains(field, pattern)` / `starts_with` / `ends_with` | String matching (backend-defined semantics). |
| `Filter::and(filters)` / `Filter::or(filters)` | Boolean composition; **must have at least one child**. |
| `Filter::negate(filter)` (also the `!` operator) | Logical NOT of any sub-tree. |
| `Filter::on_date(field, "YYYY-MM-DD")` | Convenience: expands to a `gte`/`lt` timestamp range covering the UTC day. **Fallible** — returns an error for an invalid date. |

### Scalar values

Comparison values are flat scalars (`ScalarValue`): `Null`, `Bool`, `Int32`, `Int64`, `Float64`, `Str`, `Binary`, `Timestamp` (an ISO-8601 string; use the `Timestamp` newtype to distinguish it from a plain string). `From` impls exist for `&str`, `String`, `i32`, `i64`, `f64`, and `bool`, so `Filter::eq("status", "active")` works without explicit wrapping.

### Host-enforced limits

On the host side each constructor call builds a node of a filter tree (the WIT `filter` resource). The host rejects trees that exceed fixed complexity thresholds, regardless of backend:

| Limit | Value | Applies to |
| ----- | ----- | ---------- |
| Maximum nesting depth | 5 | `and` / `or` / `not` combinators |
| Maximum list size | 100 | `in_list` / `not_in_list` value lists |
| Minimum combinator children | 1 | `and` / `or` (empty lists rejected) |

Backends may reject more (see the backend matrix below): a filter accepted by the host can still fail at `query` time if the backend cannot evaluate it server-side.

## Sorting and pagination

- `order-by` sorts before pagination; with no sort keys, ordering is backend-defined but stable within a backend (the default backend breaks ties on document id, so pages are deterministic).
- `limit` + `continuation` is the portable pagination pattern: read a page, then pass the returned token back in the next query's `continuation`. The token is opaque and backend-specific — never construct or inspect it.
- `offset` skips documents after filter/sort. Prefer continuation tokens for paging; offsets re-scan on every call.
- The default backend caps any single page at **1000 documents**, even when `limit` is absent or larger.

## Errors

| Variant | Meaning |
| ------- | ------- |
| `no-such-store` | The named collection does not exist (backend-dependent; the default backend creates collections implicitly). |
| `access-denied` | The backend refused the operation (credentials, permissions). |
| `other(string)` | Everything else — the message names the cause (e.g. `insert` on an existing id, a filter the backend cannot translate). |

Filter-limit violations (depth, list size, empty combinator) fail at filter construction, before `query` is ever called.

## Backends implementing this interface

| Backend | Location | Notes |
| ------- | -------- | ----- |
| `DocStoreDefault` | in-tree (`wasi-docstore`) | In-memory; evaluates filters directly over the stored JSON. Dotted field paths descend into nested objects; a missing field reads as JSON `null`; ordering comparisons only match comparable types. State is process-local and lost on exit. |
| `omnia-azure-table` | [omnia-backends](https://github.com/augentic/omnia-backends) repo | Azure Table Storage over REST; translates filters to OData `$filter`. **Rejects** `contains` / `starts_with` / `ends_with` / `is_null` / `is_not_null` (Azure Table's OData subset has no string functions or null checks) rather than falling back to client-side scans. Field names must match `[A-Za-z_][A-Za-z0-9_]*`. Config: `AZURE_STORAGE_ACCOUNT`, `AZURE_STORAGE_KEY`, optional `AZURE_TABLE_ENDPOINT` (point at Azurite for local emulation). |

Writing a portable guest: stick to comparisons, set membership, and `and`/`or`/`not` composition — every backend supports those. The string and null predicates are supported by the default backend but not by Azure Table.
