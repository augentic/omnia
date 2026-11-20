# DocStore Interface Reference

Reference for the `omnia:docstore` interface (version `0.1.0`): the operations, the guest SDK mapping, the portable names and query contract every backend serves, the filter language and its host-enforced limits, and the backend matrix. The conceptual walk-through is in [Document Store](../guides/document-store.md); the authoritative WIT is [`crates/wasi-docstore/wit/docstore.wit`](../../crates/wasi-docstore/wit/docstore.wit).

- [Operations](#operations)
- [Types](#types)
- [Portable names and bodies](#portable-names-and-bodies)
- [Query contract](#query-contract)
- [Filters](#filters)
- [Sorting and pagination](#sorting-and-pagination)
- [Errors](#errors)
- [Backends implementing this interface](#backends-implementing-this-interface)

## Operations

A guest opens a collection by name and calls it through the handle `open` returns:

```wit
// interface store
open: async func(name: string) -> result<collection, error>;

// resource collection
get: async func(id: string) -> result<option<document>, error>;
get-many: async func(ids: list<string>) -> result<list<option<document>>, error>;
exists: async func(id: string) -> result<bool, error>;
put: async func(id: string, data: json, when: option<precondition>)
  -> result<result<revision, conflict>, error>;
put-many: async func(documents: list<tuple<string, json>>) -> result<_, error>;
delete: async func(id: string, when: option<revision>) -> result<result<bool, conflict>, error>;
delete-many: async func(ids: list<string>) -> result<_, error>;
query: async func(filter: filter, order-by: list<sort-field>, after: option<string>, limit: option<u32>)
  -> result<page, error>;
```

| Operation | Semantics |
| --------- | --------- |
| `open` | The handle on a collection the deployment declares. An undeclared name fails with `not-found`, here or at the handle's first call. |
| `get` | Point read by id, `none` when absent. |
| `get-many` | The document at each id, in the order given. |
| `exists` | Whether a document is stored under the id. |
| `put` | Store the body under the id, replacing any body there, and return its revision. With `when`, only while the precondition holds. |
| `put-many` | Store each body under its id. An id given twice is `invalid` and writes nothing. Each write is atomic and the batch is not. |
| `delete` | Delete the document, and with `when` only while it has that revision, and return whether one was removed. Without `when`, deleting an absent document succeeds with `false`. With it, an absent document is a `conflict` whose `current` is `none`. |
| `delete-many` | Delete the document at each id, skipping any already absent. |
| `query` | One page of the documents a filter selects (see [Query contract](#query-contract)). |

The deployment declares its collections. The default backend serves the names `DOCSTORE_COLLECTIONS` lists, and a write never creates a collection. Every backend serves the `core` world, which is the `types`, `store`, and `collection` interfaces.

The host answers `unsupported` for the calls a backend cannot express yet:

- a `put` with `matches`;
- a `delete` with a revision;
- an `array-contains` test.

Guests call these through `omnia_sdk::docstore`, which re-exports the bindings as `store`, `collection`, and `types`. `docstore::open(name)` opens a `Collection`, whose generated methods are the WIT's own. Its helpers map to the WIT as follows:

- `get_as` is `get`, with the body decoded as `T`. `Document::decode` decodes a document `get` returned.
- `insert` is `put` with `absent`. A conflict is `Ok(Err(Exists))`, an outcome rather than an error.
- `upsert` is `put` with no precondition.
- `remove` is `delete` with no revision, and answers whether a document was removed.
- `find` is `query`, sending the `Filter` tree as groups (see [Filters](#filters)). `Query::after` is the WIT `after`, and the page's `next` is the cursor for the next page.

## Types

### `document`

| Field | Type | Meaning |
| ----- | ---- | ------- |
| `id` | `string` | Primary key within the collection. |
| `data` | `json` | The body, a JSON object. |
| `revision` | `revision` | A token for this stored state of the document. |

### `revision`

A string compared only for equality. It changes whenever the body changes. Two states that hold the same body may share one, so a document that changed and changed back may still match.

Until backends carry revisions, the host derives one from the body as `serde_json` writes it. So on the default backend, `put` returns the revision a later `get` reports. A backend that rewrites a body on read reports another.

### `precondition` and `conflict`

A `precondition` is what must hold for a write to take effect:

- `absent`: no document is stored under the id;
- `matches(revision)`: the stored document has this revision.

A write whose precondition fails returns `conflict`, whose `current` is the document found in its place, or `none` when the id is absent. A backend that cannot check a precondition atomically fails the write with `unsupported`.

### `sort-field`

`field`, named as a condition names one, plus `descending`.

### `page`

`documents`, in the order the query asked for, plus `next`, the cursor that resumes the query. Only a `none` cursor marks the last page.

## Portable names and bodies

Every backend stores a portable collection name, id, and body:

- A collection name is not empty and holds only ASCII letters, digits, `_`, and `-`.
- An id is one or more segments joined by `.`, at most 255 bytes long. Each segment is not empty and holds only ASCII letters, digits, `_`, and `-`.
- A body is a JSON object of at most 256 KiB that keeps to I-JSON (RFC 7493). Each of its top-level names starts with an ASCII letter and holds only ASCII letters, digits, and `_`.

A backend may reject any other name, id, or body as `invalid`. The host rejects a body that is not a JSON object itself, before the backend sees it.

A body reads back as the JSON value that was stored, except that:

- an object's names may come in another order;
- a field that holds `null` may be absent;
- a number may be written another way, so `1.0` may read back as `1`, though never `1` as `1.0`.

## Query contract

The [filter rules](#filters) define what a query selects. Its matches order by the first field of `order-by` as `lt` orders its values, then by each next field, with `false` before `true`. Where a document lacks an `order-by` field, or the field's values differ in type, the backend decides the order. Every backend serves a portable query, and serves any other query as written or fails it with `unsupported`. It never ignores part of a query and returns a page the full query would not. A portable query:

- names only top-level fields, each of which every document stores with one type of value;
- uses only `eq`, `lt`, `lte`, `gt`, `gte`, and `in`, negated or not;
- makes at most 15 comparisons, counting each value of an `in`;
- has an empty `order-by`.

Through the SDK that is a `Filter::Compare` with a value other than `null`, `in_list`, and `not_in_list`, combined with `and`, `or`, and `not`. Its `limit` and `Page::next` cursor lead page by page until every match is returned once.

A zero `limit` fails with `invalid`. The rest is optional, and a backend serves each feature as defined or fails the query with `unsupported`:

- the text tests `contains`, `starts-with`, and `ends-with`;
- the `null` test, a comparison with `null`, and a list that holds `null`;
- a dotted field path;
- `array-contains`;
- `order-by`.

## Filters

A filter is a list of groups. A document matches when every group holds, and a group holds when at least one of its conditions does. So `[[a], [b, c]]` reads `a and (b or c)`, and an empty filter matches every document.

A `condition` names a `field`, a `test`, and whether it is `negated`. A negated condition holds for the documents that fail the test. The field is a top-level name, or names joined by `.` that reach into nested objects.

| Test | Holds when the field |
| ---- | -------------------- |
| `eq(value)` | equals the value. |
| `lt`, `lte`, `gt`, `gte` | orders before or after the value as named. A boolean has no order, so one given a boolean is `invalid`. |
| `in(list<value>)` | equals one of the values, of which there are 1 to 100. |
| `starts-with`, `ends-with`, `contains` | is text that starts with, ends with, or holds the given text. |
| `array-contains(value)` | is an array, and one of its elements equals the value. |
| `null` | is absent, or holds `null`. |

A `value` is a `boolean`, an `integer` (`s64`), a `number` (any finite `f64`), or `text`. Tests compare by these rules:

- Numbers compare by value, whichever case carries them.
- Text compares by Unicode code point, so `a` and `A` differ.
- A field that is absent or holds `null` passes `null` and fails every other test.
- A field that holds a boolean, a number, or text fails a test given a value of another type.
- What a test other than `array-contains` makes of a field that holds an array or an object is the backend's to decide.

### Guest SDK filters

On the guest side, build a filter with the `Filter` constructors from `omnia_sdk::docstore` (defined in [`crates/sdk/src/docstore.rs`](../../crates/sdk/src/docstore.rs)):

| Constructor | Predicate |
| ----------- | --------- |
| `Filter::eq(field, value)` / `ne` / `gt` / `gte` / `lt` / `lte` | Comparison against a scalar (`Filter::cmp` takes an explicit `ComparisonOp`). Portable with a value other than `null`. |
| `Filter::in_list(field, values)` / `not_in_list` | Set membership. Portable. |
| `Filter::is_null(field)` / `is_not_null(field)` | Null/missing checks. Optional. |
| `Filter::contains(field, pattern)` / `starts_with` / `ends_with` | String matching. Optional. |
| `Filter::array_contains(field, value)` | Array membership: the field is an array holding the value. Optional. |
| `Filter::and(filters)` / `Filter::or(filters)` | Boolean composition; **must have at least one child**. Portable. |
| `Filter::negate(filter)` (also the `!` operator) | Logical NOT of any sub-tree. Portable. |
| `Filter::on_date(field, "YYYY-MM-DD")` | Convenience: expands to a `gte`/`lt` timestamp range covering the UTC day. **Fallible** — returns an error for an invalid date. |

`Collection::find` writes the tree as groups before it reaches the host:

- each `not` is pushed down to the fields by De Morgan's laws;
- `ne`, `not_in_list`, and `is_not_null` become negated `eq`, `in`, and `null` tests;
- an equality with `null` becomes the `null` test, and a list that holds `null` becomes a `null` test or an `in`;
- each `or` is distributed over the `and`s inside it.

So any nesting of `and`, `or`, and `not` reaches every backend as the same groups.

### Scalar values

Comparison values are flat scalars (`ScalarValue`): `Null`, `Bool`, `Int32`, `Int64`, `Float64`, `Str`, `Timestamp` (an ISO-8601 string; use the `Timestamp` newtype to distinguish it from a plain string). `From` impls exist for `&str`, `String`, `i32`, `i64`, `f64`, and `bool`, so `Filter::eq("status", "active")` works without explicit wrapping.

The SDK sends `Bool` as `boolean`, `Int32` and `Int64` as `integer`, `Float64` as `number`, and `Str` and `Timestamp` as `text`. A `Timestamp` therefore compares as text, which orders ISO-8601 instants in the same zone correctly.

### Reference semantics

`Filter::matches` and `SortField::compare` in `omnia_sdk::docstore` apply a filter and a sort field to JSON documents a guest already holds. Their rustdoc states the rules: dotted field paths, a missing field reading as `null`, ordering only between comparable types, and a fixed type rank for sorting. They follow the [filter rules](#filters), and settle what those leave to the backend in one fixed way. The default backend evaluates queries by the same rules with an evaluator of its own.

### Host-enforced limits

The host rejects a filter past these limits with `invalid`, regardless of backend:

| Limit | Value | Applies to |
| ----- | ----- | ---------- |
| Conditions in a filter | at most 100 | the conditions of every group together |
| Values in an `in` | 1 to 100 | each `in` test |
| Conditions in a group | at least 1 | each group |

It also rejects an ordering test given a boolean, and a number that is not finite.

`Filter::check` holds a `Filter` tree to the same limits once it is written as groups, and names them `Filter::MAX_CONDITIONS` and `Filter::MAX_LIST_LEN`. It adds two rules of the SDK's own:

- a tree nests at most `Filter::MAX_DEPTH` deep, which is 5;
- an `and` or `or` has at least one child.

It also refuses an ordering comparison with `null` or a boolean. `Collection::find` calls it, so through the SDK every limit fails the query with `invalid` before the query reaches the host.

## Sorting and pagination

- `order-by` sorts before paging, and is optional (see [Query contract](#query-contract)). Documents order by the first field as `lt` orders its values, then by each next field, and `false` comes before `true`.
- Documents the fields do not tell apart come in no particular order, as all do when `order-by` is empty. The default backend breaks every tie on the document id, so its pages are deterministic.
- Where a document lacks an `order-by` field, the backend decides its place and may leave it out. Where the field's values differ in type, the backend decides the order between the types.
- `limit` and `after` page a query: read a page, then pass its `next` back as the next query's `after`. The cursor is opaque and backend-specific, so never construct or inspect it. A cursor the backend did not issue fails with `invalid`.
- A document unchanged for the whole query is returned once. One written or deleted during the query may or may not be. The default backend resumes after the `order-by` values and id of the previous page's last document, so a document deleted from an earlier page shifts none of the rest.
- A page holds at most `limit` documents, and never more than **1,000**, even when `limit` is absent or larger.

## Errors

| Case | Meaning |
| ---- | ------- |
| `not-found` | The deployment does not declare the collection. An absent document is not an error: `get` returns `none`, and `delete` succeeds. |
| `denied` | The caller may not make the call, such as for credentials or permissions. |
| `invalid` | The request was rejected, such as a body that is not a JSON object, an id given twice in `put-many`, a zero `limit`, a filter past its limits, or a cursor the backend did not issue. |
| `unsupported` | A valid request this backend cannot serve, such as an optional query feature, a precondition it cannot check atomically, or an order it cannot keep. |
| `unavailable` | The backing store did not answer, or the call exceeded its bound. |
| `other` | Any other failure. |

The SDK reports each case as an `omnia_sdk::Error` coded with the case's name: a `BadGateway` for `unavailable`, and a `ServerError` for the rest. `Collection::remove` answers `false` for an absent document.

## Backends implementing this interface

| Backend | Location | Notes |
| ------- | -------- | ----- |
| `DocStoreDefault` | in-tree (`wasi-docstore`) | In-memory; serves the collections `DOCSTORE_COLLECTIONS` declares (comma-separated, default `default`), and every query feature the host passes it, by evaluating filters directly over the stored JSON. State is process-local and lost on exit. |
| `omnia-azure-table` | [omnia-backends](https://github.com/augentic/omnia-backends) repo | Azure Table Storage over REST. Serves the portable query by translating filters to an OData `$filter`, and fails every optional feature with `unsupported` rather than scanning client-side. Azure Table has no string functions or null checks, cannot compare with `null`, stores a nested value as a string, and has no server-side sort. A `Timestamp` reaches it as text, so it compares as a string rather than as an `Edm.DateTime`. It rewrites a body on read, so a read reports another revision than the `put` returned. Config: `AZURE_STORAGE_ACCOUNT`, `AZURE_STORAGE_KEY`, optional `AZURE_TABLE_ENDPOINT` (point at Azurite for local emulation). |

Writing a portable guest: keep to the [portable names and bodies](#portable-names-and-bodies) and the portable query of the [query contract](#query-contract), and the guest runs unchanged on every backend. An optional feature runs on the default backend, and fails with `unsupported` on a backend that cannot serve it.
