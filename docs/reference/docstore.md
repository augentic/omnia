# DocStore Interface Reference

Reference for the `omnia:docstore` interface (version `0.1.0`): the operations, the guest SDK types, the portable names and query contract every backend serves, the filter language and its host-enforced limits, and the backend matrix. The conceptual walk-through is in [Document Store](../guides/document-store.md); the authoritative WIT is [`crates/wasi-docstore/wit/docstore.wit`](../../crates/wasi-docstore/wit/docstore.wit).

- [Operations](#operations)
- [Types](#types)
- [Portable names and bodies](#portable-names-and-bodies)
- [Query contract](#query-contract)
- [Filters](#filters)
- [Sorting and pagination](#sorting-and-pagination)
- [Errors](#errors)
- [Backends implementing this interface](#backends-implementing-this-interface)

## Operations

```wit
get: async func(collection: string, id: string) -> result<option<document>, error>;
insert: async func(collection: string, doc: document) -> result<result<_, already-exists>, error>;
put: async func(collection: string, doc: document) -> result<_, error>;
delete: async func(collection: string, id: string) -> result<bool, error>;
query: async func(collection: string, options: query-options) -> result<query-result, error>;
```

| Operation | Semantics |
| --------- | --------- |
| `get` | Point read by id → `option<document>` (`none` when absent). |
| `insert` | Create; an id already stored returns `already-exists` and leaves that document untouched. |
| `put` | Unconditional upsert (create or replace). |
| `delete` | Remove by id → `bool` (whether anything was deleted). |
| `query` | Filtered read with sorting and pagination (see below). |

Collections are named by string and created implicitly on first write; there is no open/close handshake and no collection resource.

Guests normally call these through the `omnia_sdk::DocumentDb` trait, which `omnia_sdk::WasiProvider` implements on `wasm32` over the raw bindings. Those bindings are `omnia_wasi_docstore::store::{get, insert, put, delete, query}`, taking the WIT types in `omnia_wasi_docstore::types`. The trait renames the WIT shapes:

- `query-options` is `omnia_sdk::docstore::Query`, whose `after` is the WIT `continuation`.
- `query-result` is an `omnia_sdk::capability::Page<Document>`, whose `items` are the `documents` and whose `next` is the returned `continuation`.
- `already-exists` is `Ok(Err(Exists))`, an outcome rather than an error.

## Types

### `document`

| Field | Type | Meaning |
| ----- | ---- | ------- |
| `id` | `string` | Primary key within the collection. |
| `data` | `list<u8>` | JSON-serialized document body. The guest serializes (`serde_json`); the backend stores or translates as needed. |

### `already-exists`

`id` (the `string` id of the document `insert` left in place).

### `query-options`

| Field | Type | Meaning |
| ----- | ---- | ------- |
| `filter` | `option<filter>` | Filter tree; `none` matches everything. |
| `order-by` | `list<sort-field>` | Sort keys, first key wins, then the next. |
| `limit` | `option<u32>` | Maximum documents to return in this page. Zero is `invalid`. |
| `offset` | `option<u32>` | Skip this many documents after filter/sort. |
| `continuation` | `option<string>` | Opaque token from a previous `query-result`; `none` starts from the beginning. |

### `sort-field`

`field` (path into the document JSON) plus `descending` (`bool`, default ascending).

### `query-result`

`documents` (the page) plus `continuation` (`option<string>` — present when more pages exist; pass it back unchanged).

## Portable names and bodies

Every backend stores a portable collection name, id, and body:

- A collection name is 3 to 63 ASCII letters and digits, starting with a letter.
- An id follows the portable-key rule of `omnia:keyvalue`. It is not empty, holds only ASCII letters, digits, `.`, `_`, and `-`, and neither starts nor ends with `.`.
- A body is a JSON object.

A filter or sort field is portable when it is a top-level name matching `[A-Za-z_][A-Za-z0-9_]*`. A backend may reject any other name, id, or body as `invalid`.

## Query contract

`Filter::matches` and `SortField::compare` define what a query selects and how it orders the matches (see [Reference semantics](#reference-semantics)). Every backend serves the portable subset:

- comparisons with a value other than `null`;
- set membership, `in_list` and `not_in_list`;
- `and`, `or`, and `not`;
- `limit`, with the `Page::next` cursor leading to the next page until every match is returned once.

A zero `limit` fails with `invalid`. A missing field reads as `null`, so a portable query names fields every document stores.

The rest is optional. A backend serves each optional feature as `Filter::matches` and `SortField::compare` define, or fails the query with `unsupported`. It never ignores one and returns a page the full query would not:

- the string predicates `contains`, `starts_with`, and `ends_with`;
- the null predicates `is_null` and `is_not_null`, and a comparison with `null`;
- a dotted field path;
- `order_by`;
- `offset`.

## Filters

Filters reference fields *inside* the document JSON and compose into trees. On the guest side, build them with the `Filter` constructors from `omnia_sdk::docstore` (defined in [`crates/sdk/src/capabilities/docstore.rs`](../../crates/sdk/src/capabilities/docstore.rs)):

| Constructor | Predicate |
| ----------- | --------- |
| `Filter::eq(field, value)` / `ne` / `gt` / `gte` / `lt` / `lte` | Comparison against a scalar (`Filter::cmp` takes an explicit `ComparisonOp`). Portable with a value other than `null`. |
| `Filter::in_list(field, values)` / `not_in_list` | Set membership. Portable. |
| `Filter::is_null(field)` / `is_not_null(field)` | Null/missing checks. Optional. |
| `Filter::contains(field, pattern)` / `starts_with` / `ends_with` | String matching. Optional. |
| `Filter::and(filters)` / `Filter::or(filters)` | Boolean composition; **must have at least one child**. Portable. |
| `Filter::negate(filter)` (also the `!` operator) | Logical NOT of any sub-tree. Portable. |
| `Filter::on_date(field, "YYYY-MM-DD")` | Convenience: expands to a `gte`/`lt` timestamp range covering the UTC day. **Fallible** — returns an error for an invalid date. |

### Scalar values

Comparison values are flat scalars (`ScalarValue`): `Null`, `Bool`, `Int32`, `Int64`, `Float64`, `Str`, `Binary`, `Timestamp` (an ISO-8601 string; use the `Timestamp` newtype to distinguish it from a plain string). `From` impls exist for `&str`, `String`, `i32`, `i64`, `f64`, and `bool`, so `Filter::eq("status", "active")` works without explicit wrapping.

### Reference semantics

`Filter::matches` and `SortField::compare` in `omnia_sdk::docstore` define how a filter selects a JSON document and how a sort field orders two. Their rustdoc states the rules: dotted field paths, a missing field reading as `null`, ordering only between comparable types, and a fixed type rank for sorting. The default backend evaluates queries by the same rules, and `omnia_test::guest::MemoryDocs` calls the two functions directly. A provider that evaluates queries itself calls them too, rather than writing its own evaluator.

### Host-enforced limits

On the host side each constructor call builds a node of a filter tree (the WIT `filter` resource). The host rejects trees that exceed fixed complexity thresholds, regardless of backend:

| Limit | Value | Applies to |
| ----- | ----- | ---------- |
| Maximum nesting depth | 5 | `and` / `or` / `not` combinators |
| Maximum list size | 100 | `in_list` / `not_in_list` value lists |
| Minimum list size | 1 | `in_list` / `not_in_list` (empty lists rejected) |
| Minimum combinator children | 1 | `and` / `or` (empty lists rejected) |

`Filter::check` holds a filter to the same limits, which it names `Filter::MAX_DEPTH` and `Filter::MAX_LIST_LEN`. `DocumentDb::query` calls it before the query reaches the host, so through the SDK a filter past a limit fails the query with `invalid`. The WIT constructors return no error, so a guest on the raw bindings that passes a limit traps.

## Sorting and pagination

- `order-by` sorts before pagination. It is optional (see [Query contract](#query-contract)), and with no sort keys the order is backend-defined but stable within a backend (the default backend orders by document id, so pages are deterministic).
- `limit` + `continuation` is the portable pagination pattern: read a page, then pass the returned token back in the next query's `continuation`. The token is opaque and backend-specific — never construct or inspect it. A malformed token fails with `invalid`.
- `offset` skips documents after filter/sort. It is optional, and continuation tokens are the portable way to page; offsets re-scan on every call.
- The default backend caps any single page at **1000 documents**, even when `limit` is absent or larger.

## Errors

| Case | Meaning |
| ---- | ------- |
| `not-found` | The collection does not exist. An absent document is not an error: `get` returns `none` and `delete` returns `false`. |
| `denied` | The caller may not use the collection, such as for credentials or permissions. |
| `invalid` | The request was rejected, such as a body that is not JSON, a zero `limit`, a filter past its limits, or a malformed continuation. |
| `unsupported` | A valid request this backend cannot serve, such as an optional query feature. |
| `unavailable` | The backing store did not answer, or the call exceeded its bound. |
| `other` | Any other failure. |

`omnia_sdk::capability::ErrorKind` has one kind per case, and the SDK maps each case to its kind. A filter past the [host-enforced limits](#host-enforced-limits) fails with `invalid` through the SDK, and traps a guest on the raw bindings.

## Backends implementing this interface

| Backend | Location | Notes |
| ------- | -------- | ----- |
| `DocStoreDefault` | in-tree (`wasi-docstore`) | In-memory; serves every query feature by evaluating filters directly over the stored JSON. State is process-local and lost on exit. |
| `omnia-azure-table` | [omnia-backends](https://github.com/augentic/omnia-backends) repo | Azure Table Storage over REST. Serves the portable subset by translating filters to an OData `$filter`, and fails every optional feature with `unsupported` rather than scanning client-side. Azure Table has no string functions or null checks, cannot compare with `null`, stores a nested value as a string, and has no server-side sort or skip. Config: `AZURE_STORAGE_ACCOUNT`, `AZURE_STORAGE_KEY`, optional `AZURE_TABLE_ENDPOINT` (point at Azurite for local emulation). |

Writing a portable guest: keep to the [portable names and bodies](#portable-names-and-bodies) and the portable subset of the [query contract](#query-contract), and the guest runs unchanged on every backend. An optional feature runs on the default backend, and fails with `unsupported` on a backend that cannot serve it.
