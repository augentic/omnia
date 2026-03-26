# wasi-jsondb

## 1. WIT Definition

```wit
package wasi:jsondb@0.1.0;

interface types {
    variant error {
        no-such-store,
        access-denied,
        other(string),
    }

    variant scalar-value {
        null,
        boolean(bool),
        int32(s32),
        int64(s64),
        float64(f64),
        str(string),
        binary(list<u8>),
        timestamp(string),
    }

    enum comparison-op {
        eq,
        ne,
        gt,
        gte,
        lt,
        lte,
    }

    /// Host-managed filter tree. The guest builds filters by calling
    /// static constructors; the host assembles a native recursive
    /// tree internally.
    resource filter {
        compare: static func(field: string, op: comparison-op, value: scalar-value) -> filter;
        in-list: static func(field: string, values: list<scalar-value>) -> filter;
        not-in-list: static func(field: string, values: list<scalar-value>) -> filter;
        is-null: static func(field: string) -> filter;
        is-not-null: static func(field: string) -> filter;
        contains: static func(field: string, pattern: string) -> filter;
        starts-with: static func(field: string, pattern: string) -> filter;
        ends-with: static func(field: string, pattern: string) -> filter;
        and: static func(filters: list<filter>) -> filter;
        or: static func(filters: list<filter>) -> filter;
        not: static func(inner: filter) -> filter;
    }

    record sort-field {
        field: string,
        descending: bool,
    }

    record query-options {
        filter: option<filter>,
        order-by: list<sort-field>,
        limit: option<u32>,
        offset: option<u32>,
        /// Opaque pagination token. None = start from beginning.
        /// Returned by a previous query-result; pass it back to
        /// fetch the next page. Semantics are backend-dependent.
        continuation: option<string>,
    }

    record document {
        id: string,
        /// JSON-serialized document body.
        data: list<u8>,
    }

    record query-result {
        documents: list<document>,
        continuation: option<string>,
    }
}

interface store {
    use types.{document, query-options, query-result, error};

    get: async func(collection: string, id: string) -> result<option<document>, error>;
    insert: async func(collection: string, doc: document) -> result<_, error>;
    put: async func(collection: string, doc: document) -> result<_, error>;
    delete: async func(collection: string, id: string) -> result<bool, error>;
    query: async func(collection: string, options: query-options) -> result<query-result, error>;
}

world imports {
    import types;
    import store;
}
```

### Design rationale

**Documents are `list<u8>` (JSON bytes)** -- WIT cannot express recursive value types.
The guest serializes with `serde_json`, the host deserializes to whatever the backend
needs (BSON for MongoDB/PoloDB, entity properties for Azure Table Storage). This is the
same pattern `wasi-keyvalue` uses for values.

**Filters are a WIT resource** -- The guest calls static constructors (`Filter::compare`,
`Filter::and`, etc.) that cross the WASM boundary. Each call creates a node in a recursive
`FilterTree` enum on the host side. The resource approach is self-documenting, type-safe
(you cannot construct an invalid filter), and avoids both the recursive-type limitation
of WIT and the complexity of arena-based representations.

**Scalar values are flat** -- Filter comparisons only need scalars (you never filter by
`where field = {nested_object}`). This sidesteps value-type recursion entirely.

**Host-enforced filter limits** -- The host rejects filter trees that exceed fixed
complexity thresholds, regardless of backend:

| Limit | Value | Applies to |
|-------|-------|------------|
| Max nesting depth | 5 | `and`, `or`, `not` combinators |
| Max in-list size | 100 | `in-list`, `not-in-list` value lists |
| Min combinator children | 1 | `and`, `or` (empty lists rejected) |

These limits protect the host from unbounded memory and CPU consumption when
translating filters to backend-native queries (BSON, OData, SQL). A depth of 5
covers all practical filter patterns; if a guest needs deeper nesting the query
design should be reconsidered. Backends may impose additional constraints (e.g. the
PoloDB default backend rejects `starts-with` and `ends-with`, and caps query results
at 1000 documents when no explicit limit is set).

**No `collection` resource** -- "Opening" a collection is trivial in all three backends
(MongoDB returns a lightweight handle from its pooled client, Azure just needs the table
name in a URL, PoloDB resolves a name). The expensive resources (MongoDB connection pool,
PoloDB database handle) are managed at the backend level via `Backend::connect_with` at
host startup. A collection resource would add a resource table entry and an extra boundary
crossing per operation for something that is just a string lookup.

**No separate `date` type** -- None of the three backends (Azure Table Storage, MongoDB,
PoloDB) have a native date-only type; they all use datetime/timestamp. The `timestamp`
scalar covers datetime filtering, `str` covers date-as-string comparisons, and a
guest-side `Filter::on_date` convenience method handles the range-expansion pattern.

**Flat `store` interface** -- Five operations: `get` (point read by ID), `insert`
(create new, fail if exists), `put` (unconditional upsert), `delete` (remove by ID),
`query` (filtered read with pagination). `insert` vs `put` is a meaningful distinction --
all three backends support insert-with-conflict natively (Azure returns 409, MongoDB
returns duplicate key error). `get` returns `document` (not just data) for consistency
with `query`.

---

## 2. SDK Capability -- `DocumentStore`

These types and the trait live in `omnia-sdk`. They are platform-agnostic -- no
`#[cfg(target_arch)]` guards, no dependency on `omnia-wasi-jsondb` guest
bindings. Domain crates import these and use them in handler bounds.

### Types

```rust
// omnia-sdk/src/document_store.rs

/// Scalar values for filter comparisons.
#[derive(Debug, Clone, PartialEq)]
pub enum ScalarValue {
    Null,
    Bool(bool),
    Int32(i32),
    Int64(i64),
    Float64(f64),
    Str(String),
    Binary(Vec<u8>),
    Timestamp(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComparisonOp {
    Eq,
    Ne,
    Gt,
    Gte,
    Lt,
    Lte,
}

/// A filter expression tree. Recursive -- this is a plain Rust type,
/// not constrained by WIT's type system.
#[derive(Debug, Clone)]
pub enum Filter {
    Compare { field: String, op: ComparisonOp, value: ScalarValue },
    InList { field: String, values: Vec<ScalarValue> },
    NotInList { field: String, values: Vec<ScalarValue> },
    IsNull(String),
    IsNotNull(String),
    Contains { field: String, pattern: String },
    StartsWith { field: String, pattern: String },
    EndsWith { field: String, pattern: String },
    And(Vec<Filter>),
    Or(Vec<Filter>),
    Not(Box<Filter>),
}

#[derive(Debug, Clone, Default)]
pub struct SortField {
    pub field: String,
    pub descending: bool,
}

#[derive(Debug, Clone, Default)]
pub struct QueryOptions {
    pub filter: Option<Filter>,
    pub order_by: Vec<SortField>,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
    pub continuation: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Document {
    pub id: String,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, Default)]
pub struct QueryResult {
    pub documents: Vec<Document>,
    pub continuation: Option<String>,
}
```

### Convenience constructors

```rust
// From impls for ergonomic value construction
impl From<&str> for ScalarValue {
    fn from(s: &str) -> Self { Self::Str(s.to_string()) }
}
impl From<String> for ScalarValue {
    fn from(s: String) -> Self { Self::Str(s) }
}
impl From<i32> for ScalarValue {
    fn from(v: i32) -> Self { Self::Int32(v) }
}
impl From<i64> for ScalarValue {
    fn from(v: i64) -> Self { Self::Int64(v) }
}
impl From<f64> for ScalarValue {
    fn from(v: f64) -> Self { Self::Float64(v) }
}
impl From<bool> for ScalarValue {
    fn from(v: bool) -> Self { Self::Bool(v) }
}

/// Newtype for timestamp strings, so Filter::gte("ts", Timestamp("..."))
/// is unambiguous vs a plain string comparison.
pub struct Timestamp(pub String);

impl From<Timestamp> for ScalarValue {
    fn from(t: Timestamp) -> Self { Self::Timestamp(t.0) }
}

impl Filter {
    pub fn eq(field: &str, val: impl Into<ScalarValue>) -> Self {
        Self::Compare { field: field.to_string(), op: ComparisonOp::Eq, value: val.into() }
    }
    pub fn ne(field: &str, val: impl Into<ScalarValue>) -> Self {
        Self::Compare { field: field.to_string(), op: ComparisonOp::Ne, value: val.into() }
    }
    pub fn gt(field: &str, val: impl Into<ScalarValue>) -> Self {
        Self::Compare { field: field.to_string(), op: ComparisonOp::Gt, value: val.into() }
    }
    pub fn gte(field: &str, val: impl Into<ScalarValue>) -> Self {
        Self::Compare { field: field.to_string(), op: ComparisonOp::Gte, value: val.into() }
    }
    pub fn lt(field: &str, val: impl Into<ScalarValue>) -> Self {
        Self::Compare { field: field.to_string(), op: ComparisonOp::Lt, value: val.into() }
    }
    pub fn lte(field: &str, val: impl Into<ScalarValue>) -> Self {
        Self::Compare { field: field.to_string(), op: ComparisonOp::Lte, value: val.into() }
    }
    pub fn in_list(field: &str, vals: impl IntoIterator<Item = impl Into<ScalarValue>>) -> Self {
        Self::InList {
            field: field.to_string(),
            values: vals.into_iter().map(Into::into).collect(),
        }
    }
    pub fn not_in_list(field: &str, vals: impl IntoIterator<Item = impl Into<ScalarValue>>) -> Self {
        Self::NotInList {
            field: field.to_string(),
            values: vals.into_iter().map(Into::into).collect(),
        }
    }
    pub fn is_null(field: &str) -> Self { Self::IsNull(field.to_string()) }
    pub fn is_not_null(field: &str) -> Self { Self::IsNotNull(field.to_string()) }
    pub fn contains(field: &str, pattern: &str) -> Self {
        Self::Contains { field: field.to_string(), pattern: pattern.to_string() }
    }
    pub fn starts_with(field: &str, pattern: &str) -> Self {
        Self::StartsWith { field: field.to_string(), pattern: pattern.to_string() }
    }
    pub fn ends_with(field: &str, pattern: &str) -> Self {
        Self::EndsWith { field: field.to_string(), pattern: pattern.to_string() }
    }
    pub fn and(filters: impl IntoIterator<Item = Filter>) -> Self {
        Self::And(filters.into_iter().collect())
    }
    pub fn or(filters: impl IntoIterator<Item = Filter>) -> Self {
        Self::Or(filters.into_iter().collect())
    }
    pub fn not(inner: Filter) -> Self {
        Self::Not(Box::new(inner))
    }
    /// Date convenience: expands to gte(T00:00:00Z) AND lt(next day T00:00:00Z).
    pub fn on_date(field: &str, iso_date: &str) -> Self {
        // actual implementation parses iso_date and increments the day
        Self::And(vec![
            Self::gte(field, Timestamp(format!("{iso_date}T00:00:00Z"))),
            Self::lt(field, Timestamp(format!("{iso_date}T00:00:00Z"))), // incremented in real impl
        ])
    }
}
```

### Trait

Follows the same pattern as `TableStore`, `StateStore`, etc. in `capabilities.rs`.
The `#[cfg(target_arch = "wasm32")]` default impls delegate to the guest module of
`omnia-wasi-jsondb` (see section 3).

```rust
// omnia-sdk/src/capabilities.rs

pub trait DocumentStore: Send + Sync {
    #[cfg(not(target_arch = "wasm32"))]
    fn get(
        &self, store: &str, id: &str,
    ) -> impl Future<Output = Result<Option<Document>>> + Send;

    #[cfg(not(target_arch = "wasm32"))]
    fn insert(&self, store: &str, doc: &Document) -> impl Future<Output = Result<()>> + Send;

    #[cfg(not(target_arch = "wasm32"))]
    fn put(&self, store: &str, doc: &Document) -> impl Future<Output = Result<()>> + Send;

    #[cfg(not(target_arch = "wasm32"))]
    fn delete(&self, store: &str, id: &str) -> impl Future<Output = Result<bool>> + Send;

    #[cfg(not(target_arch = "wasm32"))]
    fn query(
        &self, store: &str, options: QueryOptions,
    ) -> impl Future<Output = Result<QueryResult>> + Send;

    #[cfg(target_arch = "wasm32")]
    fn get(
        &self, store: &str, id: &str,
    ) -> impl Future<Output = Result<Option<Document>>> + Send {
        async move {
            omnia_wasi_jsondb::store::get(store, id).await
        }
    }

    #[cfg(target_arch = "wasm32")]
    fn insert(&self, store: &str, doc: &Document) -> impl Future<Output = Result<()>> + Send {
        async move {
            omnia_wasi_jsondb::store::insert(store, doc).await
        }
    }

    #[cfg(target_arch = "wasm32")]
    fn put(&self, store: &str, doc: &Document) -> impl Future<Output = Result<()>> + Send {
        async move {
            omnia_wasi_jsondb::store::put(store, doc).await
        }
    }

    #[cfg(target_arch = "wasm32")]
    fn delete(&self, store: &str, id: &str) -> impl Future<Output = Result<bool>> + Send {
        async move {
            omnia_wasi_jsondb::store::delete(store, id).await
        }
    }

    #[cfg(target_arch = "wasm32")]
    fn query(
        &self, store: &str, options: QueryOptions,
    ) -> impl Future<Output = Result<QueryResult>> + Send {
        async move {
            omnia_wasi_jsondb::store::query(store, options).await
        }
    }
}
```

### Domain handler usage

This is what a domain crate author writes. No WASM awareness, no WIT types.

```rust
// crates/my_domain/src/handlers/search.rs

use omnia_sdk::{DocumentStore, Result};
use omnia_sdk::document_store::{Filter, QueryOptions, SortField, Timestamp};

async fn handle<P>(
    owner: &str, request: SearchRequest, provider: &P,
) -> Result<Reply<Vec<Item>>>
where
    P: Config + DocumentStore,
{
    let filter = Filter::and([
        Filter::eq("status", "active"),
        Filter::gte("created_at", Timestamp("2024-01-01T00:00:00Z".into())),
        Filter::or([
            Filter::eq("category", "electronics"),
            Filter::eq("category", "books"),
        ]),
    ]);

    let result = DocumentStore::query(provider, "items", QueryOptions {
        filter: Some(filter),
        order_by: vec![SortField { field: "created_at".into(), descending: true }],
        limit: Some(25),
        ..Default::default()
    }).await?;

    let items: Vec<Item> = result.documents.iter()
        .map(|doc| serde_json::from_slice(&doc.data))
        .collect::<serde_json::Result<_>>()
        .map_err(|e| bad_request!("malformed document: {e}"))?;

    Ok(Reply::ok(items))
}
```

### Native test mock

```rust
// tests/provider.rs

struct MockProvider {
    documents: HashMap<String, Vec<Document>>,
}

impl DocumentStore for MockProvider {
    async fn query(&self, store: &str, options: QueryOptions) -> Result<QueryResult> {
        let docs = self.documents.get(store).cloned().unwrap_or_default();
        Ok(QueryResult { documents: docs, continuation: None })
    }
    // ... get, insert, put, delete
}
```

---

## 3. Rust to WIT Conversion (guest module)

The conversion from SDK types to WIT types lives in the **guest module** of
`omnia-wasi-jsondb`. This follows the same pattern as
`omnia-wasi-keyvalue/src/guest/cache.rs` -- the guest module wraps raw WIT bindings
with a higher-level API that accepts SDK types.

The `capabilities.rs` wasm32 default impls just delegate to this module in one or two
lines (see section 2 trait definition above).

### Guest module structure

```
crates/wasi-jsondb/src/
  guest.rs              # wit_bindgen::generate! + re-exports
  guest/
    mod.rs
    store.rs            # public API: get, insert, put, delete, query
    convert.rs          # private: to_wit_filter, to_wit_value, to_wit_op, etc.
```

### Type zones

There are three copies of each type (ScalarValue, ComparisonOp, SortField, etc.),
living in three zones:

| Zone | Generated by | Target |
|------|-------------|--------|
| SDK types (`omnia_sdk::document_store`) | Hand-written | Platform-agnostic, used by domain crates |
| Guest WIT types (`omnia_wasi_jsondb::types`) | `wit_bindgen::generate!` | wasm32 only |
| Host WIT types (`omnia_wasi_jsondb::host::generated`) | `wasmtime::component::bindgen!` | Native only |

Guest and host WIT types are structurally identical (generated from the same WIT file).
The component model ABI handles serialization between them automatically -- no code
needed for that boundary.

Hand-written conversion code exists at **one** point: SDK types to guest WIT types,
in `convert.rs`.

### store.rs -- public API

```rust
// crates/wasi-jsondb/src/guest/store.rs

use anyhow::{Context, Result};
use omnia_sdk::document_store as sdk;

use crate::guest::store as wit_store;
mod convert;

pub async fn get(collection: &str, id: &str) -> Result<Option<sdk::Document>> {
    let result = wit_store::get(collection.into(), id.into())
        .await
        .map_err(|e| anyhow::anyhow!("get failed: {e:?}"))?;
    Ok(result.map(convert::from_wit_document))
}

pub async fn insert(collection: &str, doc: &sdk::Document) -> Result<()> {
    wit_store::insert(collection.into(), convert::to_wit_document(doc))
        .await
        .map_err(|e| anyhow::anyhow!("insert failed: {e:?}"))
}

pub async fn put(collection: &str, doc: &sdk::Document) -> Result<()> {
    wit_store::put(collection.into(), convert::to_wit_document(doc))
        .await
        .map_err(|e| anyhow::anyhow!("put failed: {e:?}"))
}

pub async fn delete(collection: &str, id: &str) -> Result<bool> {
    wit_store::delete(collection.into(), id.into())
        .await
        .map_err(|e| anyhow::anyhow!("delete failed: {e:?}"))
}

pub async fn query(collection: &str, options: sdk::QueryOptions) -> Result<sdk::QueryResult> {
    let wit_options = convert::to_wit_query_options(options);
    let result = wit_store::query(collection.into(), wit_options)
        .await
        .map_err(|e| anyhow::anyhow!("query failed: {e:?}"))?;
    Ok(convert::from_wit_query_result(result))
}
```

### convert.rs -- SDK to WIT type mappings

All functions are `pub(super)` -- visible to `store.rs`, invisible outside the module.

```rust
// crates/wasi-jsondb/src/guest/convert.rs

use omnia_sdk::document_store as sdk;
use crate::guest::types as wit;

// --- SDK -> WIT (for outbound calls) ---

pub(super) fn to_wit_filter(filter: sdk::Filter) -> wit::Filter {
    match filter {
        sdk::Filter::Compare { field, op, value } => {
            wit::Filter::compare(&field, to_wit_op(op), &to_wit_value(value))
        }
        sdk::Filter::InList { field, values } => {
            let vals: Vec<_> = values.into_iter().map(to_wit_value).collect();
            wit::Filter::in_list(&field, &vals)
        }
        sdk::Filter::NotInList { field, values } => {
            let vals: Vec<_> = values.into_iter().map(to_wit_value).collect();
            wit::Filter::not_in_list(&field, &vals)
        }
        sdk::Filter::IsNull(field) => wit::Filter::is_null(&field),
        sdk::Filter::IsNotNull(field) => wit::Filter::is_not_null(&field),
        sdk::Filter::Contains { field, pattern } => wit::Filter::contains(&field, &pattern),
        sdk::Filter::StartsWith { field, pattern } => wit::Filter::starts_with(&field, &pattern),
        sdk::Filter::EndsWith { field, pattern } => wit::Filter::ends_with(&field, &pattern),
        sdk::Filter::And(children) => {
            let wit_children: Vec<_> = children.into_iter().map(to_wit_filter).collect();
            wit::Filter::and(wit_children)
        }
        sdk::Filter::Or(children) => {
            let wit_children: Vec<_> = children.into_iter().map(to_wit_filter).collect();
            wit::Filter::or(wit_children)
        }
        sdk::Filter::Not(inner) => {
            wit::Filter::not(to_wit_filter(*inner))
        }
    }
}

pub(super) fn to_wit_op(op: sdk::ComparisonOp) -> wit::ComparisonOp {
    match op {
        sdk::ComparisonOp::Eq  => wit::ComparisonOp::Eq,
        sdk::ComparisonOp::Ne  => wit::ComparisonOp::Ne,
        sdk::ComparisonOp::Gt  => wit::ComparisonOp::Gt,
        sdk::ComparisonOp::Gte => wit::ComparisonOp::Gte,
        sdk::ComparisonOp::Lt  => wit::ComparisonOp::Lt,
        sdk::ComparisonOp::Lte => wit::ComparisonOp::Lte,
    }
}

pub(super) fn to_wit_value(v: sdk::ScalarValue) -> wit::ScalarValue {
    match v {
        sdk::ScalarValue::Null        => wit::ScalarValue::Null,
        sdk::ScalarValue::Bool(b)     => wit::ScalarValue::Bool(b),
        sdk::ScalarValue::Int32(i)    => wit::ScalarValue::Int32(i),
        sdk::ScalarValue::Int64(i)    => wit::ScalarValue::Int64(i),
        sdk::ScalarValue::Float64(f)  => wit::ScalarValue::Float64(f),
        sdk::ScalarValue::Str(s)      => wit::ScalarValue::Str(s),
        sdk::ScalarValue::Binary(b)   => wit::ScalarValue::Binary(b),
        sdk::ScalarValue::Timestamp(t)=> wit::ScalarValue::Timestamp(t),
    }
}

pub(super) fn to_wit_sort(s: &sdk::SortField) -> wit::SortField {
    wit::SortField { field: s.field.clone(), descending: s.descending }
}

pub(super) fn to_wit_document(d: &sdk::Document) -> wit::Document {
    wit::Document { id: d.id.clone(), data: d.data.clone() }
}

pub(super) fn to_wit_query_options(o: sdk::QueryOptions) -> wit::QueryOptions {
    wit::QueryOptions {
        filter: o.filter.map(to_wit_filter),
        order_by: o.order_by.iter().map(to_wit_sort).collect(),
        limit: o.limit,
        offset: o.offset,
        continuation: o.continuation,
    }
}

// --- WIT -> SDK (for inbound results) ---

pub(super) fn from_wit_document(d: wit::Document) -> sdk::Document {
    sdk::Document { id: d.id, data: d.data }
}

pub(super) fn from_wit_query_result(r: wit::QueryResult) -> sdk::QueryResult {
    sdk::QueryResult {
        documents: r.documents.into_iter().map(from_wit_document).collect(),
        continuation: r.continuation,
    }
}
```

### Boundary crossing trace

Given this domain code:

```rust
Filter::and([
    Filter::eq("status", "active"),
    Filter::gt("age", 18),
])
```

The guest `to_wit_filter` produces these boundary crossings:

```
to_wit_filter(And([Compare{status,Eq,active}, Compare{age,Gt,18}]))
  |
  +-- to_wit_filter(Compare{status,Eq,active})
  |     \-- WASM->Host: Filter::compare("status", Eq, Str("active"))
  |         Host stores FilterTree::Compare{status,Eq,"active"} -> handle 1
  |
  +-- to_wit_filter(Compare{age,Gt,18})
  |     \-- WASM->Host: Filter::compare("age", Gt, Int32(18))
  |         Host stores FilterTree::Compare{age,Gt,18} -> handle 2
  |
  \-- WASM->Host: Filter::and([handle 1, handle 2])
      Host removes handles 1 and 2 from resource table
      Host stores FilterTree::And([Compare{status,Eq,"active"}, Compare{age,Gt,18}])
        -> handle 3
```

Three lightweight boundary crossings. The host now has a clean recursive
`FilterTree` at handle 3. When `store.query` receives this handle inside
`query-options`, the backend gets the `FilterTree` directly for translation.

---

## 4. Host Backend -- Filter Resurrection

### Wasmtime bindgen

The host module generates Rust types from the WIT using `wasmtime::component::bindgen!`.
The `with:` clause maps the WIT `filter` resource to our `FilterProxy` type --
this is how wasmtime knows what to store in the resource table when the guest calls
filter constructors.

```rust
// crates/wasi-jsondb/src/host.rs

mod generated {
    #![allow(missing_docs)]

    pub use super::FilterProxy;

    wasmtime::component::bindgen!({
        world: "imports",
        path: "wit",
        imports: {
            default: store | tracing | trappable,
        },
        with: {
            "wasi:jsondb/types.filter": FilterProxy,
        },
        trappable_error_type: {
            "wasi:jsondb/types.error" => Error,
        },
    });
}
```

This generates host-side traits like `HostFilterWithStore` (for the `filter` resource
static methods) and `HostStoreWithStore` (for the `store` interface functions).
The `WithStore` suffix comes from the `store` option, which uses the `Accessor`
pattern instead of `&mut self`. All wasi-\* host crates in the repo use this option.

The generated traits expect `Resource<FilterProxy>` as the return type for filter
constructors and accept it as input in `query-options.filter`. Our implementation
creates `FilterProxy` values and pushes them into the resource table.

### FilterProxy and FilterTree

`FilterTree` is the internal representation of the `filter` resource -- the same way
`InMemContainer` is the internal representation of blobstore's `container` resource,
or `InMemBucket` is the internal representation of keyvalue's `bucket` resource.

`FilterProxy` is the newtype wrapper that goes in the resource table (following the
`ContainerProxy`, `BucketProxy` pattern in the codebase).

The `FilterTree` uses wasmtime-generated `ComparisonOp` and `ScalarValue` types
directly -- no conversion needed. They arrive as function parameters in the resource
constructors and go straight into `FilterTree` variants.

```rust
// crates/wasi-jsondb/src/host/resource.rs

use crate::host::generated::wasi::jsondb::types::{ScalarValue, ComparisonOp};

#[derive(Debug, Clone)]
pub enum FilterTree {
    Compare { field: String, op: ComparisonOp, value: ScalarValue },
    InList { field: String, values: Vec<ScalarValue> },
    NotInList { field: String, values: Vec<ScalarValue> },
    IsNull(String),
    IsNotNull(String),
    Contains { field: String, pattern: String },
    StartsWith { field: String, pattern: String },
    EndsWith { field: String, pattern: String },
    And(Vec<FilterTree>),
    Or(Vec<FilterTree>),
    Not(Box<FilterTree>),
}

/// Wraps FilterTree for the wasmtime resource table.
#[derive(Debug, Clone)]
pub struct FilterProxy(pub FilterTree);
```

### Host-side resource implementation

When the guest calls the WIT filter resource constructors, the host builds
`FilterTree` nodes and stores them in the resource table. The wasmtime-generated
types arrive as parameters and go straight into `FilterTree` -- no conversion.

```rust
// crates/wasi-jsondb/src/host/types_impl.rs

impl HostFilterWithStore for WasiJsonDb {
    fn compare<T>(
        accessor: &Accessor<T, Self>,
        field: String, op: ComparisonOp, value: ScalarValue,
    ) -> Result<Resource<FilterProxy>> {
        let tree = FilterTree::Compare { field, op, value };
        accessor.with(|mut store| store.get().table.push(FilterProxy(tree)))
            .map_err(|e| e.to_string())
    }

    fn and<T>(
        accessor: &Accessor<T, Self>,
        filters: Vec<Resource<FilterProxy>>,
    ) -> Result<Resource<FilterProxy>> {
        let children: Vec<FilterTree> = accessor.with(|mut store| {
            filters.into_iter()
                .map(|r| store.get().table.delete(r).map(|fp| fp.0))
                .collect::<wasmtime::Result<Vec<_>>>()
        }).map_err(|e| e.to_string())?;

        let tree = FilterTree::And(children);
        accessor.with(|mut store| store.get().table.push(FilterProxy(tree)))
            .map_err(|e| e.to_string())
    }

    fn or<T>(
        accessor: &Accessor<T, Self>,
        filters: Vec<Resource<FilterProxy>>,
    ) -> Result<Resource<FilterProxy>> {
        let children: Vec<FilterTree> = accessor.with(|mut store| {
            filters.into_iter()
                .map(|r| store.get().table.delete(r).map(|fp| fp.0))
                .collect::<wasmtime::Result<Vec<_>>>()
        }).map_err(|e| e.to_string())?;

        let tree = FilterTree::Or(children);
        accessor.with(|mut store| store.get().table.push(FilterProxy(tree)))
            .map_err(|e| e.to_string())
    }

    fn not<T>(
        accessor: &Accessor<T, Self>,
        filter: Resource<FilterProxy>,
    ) -> Result<Resource<FilterProxy>> {
        let inner = accessor.with(|mut store| store.get().table.delete(filter))
            .map_err(|e| e.to_string())?;
        let tree = FilterTree::Not(Box::new(inner.0));
        accessor.with(|mut store| store.get().table.push(FilterProxy(tree)))
            .map_err(|e| e.to_string())
    }

    // is_null, is_not_null, contains, starts_with, ends_with, in_list, not_in_list
    // all follow the same pattern as compare (leaf node, push to resource table)
}
```

When `store.query` is called and `query-options.filter` contains a resource handle,
the host pulls `FilterProxy` from the resource table and passes `FilterTree` to
the backend translator.

---

### Azure Table Storage translator

Azure Table Storage speaks OData. The translator walks the `FilterTree` and emits an
OData `$filter` string.

```rust
// crates/wasi-jsondb/src/host/azure/filter.rs

pub fn to_odata(tree: &FilterTree) -> String {
    match tree {
        FilterTree::Compare { field, op, value } => {
            let op_str = match op {
                ComparisonOp::Eq  => "eq",
                ComparisonOp::Ne  => "ne",
                ComparisonOp::Gt  => "gt",
                ComparisonOp::Gte => "ge",
                ComparisonOp::Lt  => "lt",
                ComparisonOp::Lte => "le",
            };
            format!("{field} {op_str} {}", odata_value(value))
        }
        FilterTree::InList { field, values } => {
            let parts: Vec<_> = values.iter()
                .map(|v| format!("{field} eq {}", odata_value(v)))
                .collect();
            format!("({})", parts.join(" or "))
        }
        FilterTree::NotInList { field, values } => {
            let parts: Vec<_> = values.iter()
                .map(|v| format!("{field} ne {}", odata_value(v)))
                .collect();
            format!("({})", parts.join(" and "))
        }
        FilterTree::IsNull(field) => format!("{field} eq null"),
        FilterTree::IsNotNull(field) => format!("{field} ne null"),
        FilterTree::Contains { field, pattern } => {
            format!("contains({field},'{}')", escape_odata(pattern))
        }
        FilterTree::StartsWith { field, pattern } => {
            format!("startswith({field},'{}')", escape_odata(pattern))
        }
        FilterTree::EndsWith { field, pattern } => {
            format!("endswith({field},'{}')", escape_odata(pattern))
        }
        FilterTree::And(children) => {
            let parts: Vec<_> = children.iter().map(to_odata).collect();
            parts.join(" and ")
        }
        FilterTree::Or(children) => {
            let parts: Vec<_> = children.iter().map(to_odata).collect();
            format!("({})", parts.join(" or "))
        }
        FilterTree::Not(inner) => {
            format!("not ({})", to_odata(inner))
        }
    }
}

fn odata_value(v: &ScalarValue) -> String {
    match v {
        ScalarValue::Null        => "null".to_string(),
        ScalarValue::Bool(b)     => b.to_string(),
        ScalarValue::Int32(i)    => i.to_string(),
        ScalarValue::Int64(i)    => format!("{i}L"),
        ScalarValue::Float64(f)  => format!("{f}"),
        ScalarValue::Str(s)      => format!("'{}'", s.replace('\'', "''")),
        ScalarValue::Timestamp(t)=> format!("datetime'{t}'"),
        ScalarValue::Binary(b)   => format!("X'{}'", hex::encode(b)),
    }
}

fn escape_odata(s: &str) -> String {
    s.replace('\'', "''")
}
```

**Example.** Given this domain code:

```rust
Filter::and([
    Filter::eq("PartitionKey", "customers-us"),
    Filter::gte("created_at", Timestamp("2024-01-01T00:00:00Z".into())),
    Filter::or([
        Filter::eq("status", "active"),
        Filter::eq("status", "pending"),
    ]),
])
```

Azure backend produces:

```
GET https://myaccount.table.core.windows.net/items
    ?$filter=PartitionKey eq 'customers-us'
             and created_at ge datetime'2024-01-01T00:00:00Z'
             and (status eq 'active' or status eq 'pending')
```

---

### MongoDB translator

MongoDB speaks BSON. The translator walks the `FilterTree` and builds a
`bson::Document`.

```rust
// crates/wasi-jsondb/src/host/mongodb/filter.rs

use bson::{doc, Bson, Document};

pub fn to_bson(tree: &FilterTree) -> Document {
    match tree {
        FilterTree::Compare { field, op, value } => {
            let bson_op = match op {
                ComparisonOp::Eq  => "$eq",
                ComparisonOp::Ne  => "$ne",
                ComparisonOp::Gt  => "$gt",
                ComparisonOp::Gte => "$gte",
                ComparisonOp::Lt  => "$lt",
                ComparisonOp::Lte => "$lte",
            };
            doc! { field: { bson_op: to_bson_value(value) } }
        }
        FilterTree::InList { field, values } => {
            let bson_vals: Vec<Bson> = values.iter().map(to_bson_value).collect();
            doc! { field: { "$in": bson_vals } }
        }
        FilterTree::NotInList { field, values } => {
            let bson_vals: Vec<Bson> = values.iter().map(to_bson_value).collect();
            doc! { field: { "$nin": bson_vals } }
        }
        FilterTree::IsNull(field)    => doc! { field: Bson::Null },
        FilterTree::IsNotNull(field) => doc! { field: { "$ne": Bson::Null } },
        FilterTree::Contains { field, pattern } => {
            doc! { field: { "$regex": regex_escape(pattern), "$options": "" } }
        }
        FilterTree::StartsWith { field, pattern } => {
            doc! { field: { "$regex": format!("^{}", regex_escape(pattern)), "$options": "" } }
        }
        FilterTree::EndsWith { field, pattern } => {
            doc! { field: { "$regex": format!("{}$", regex_escape(pattern)), "$options": "" } }
        }
        FilterTree::And(children) => {
            let docs: Vec<Bson> = children.iter()
                .map(|c| Bson::Document(to_bson(c)))
                .collect();
            doc! { "$and": docs }
        }
        FilterTree::Or(children) => {
            let docs: Vec<Bson> = children.iter()
                .map(|c| Bson::Document(to_bson(c)))
                .collect();
            doc! { "$or": docs }
        }
        FilterTree::Not(inner) => {
            doc! { "$nor": [to_bson(inner)] }
        }
    }
}

fn to_bson_value(v: &ScalarValue) -> Bson {
    match v {
        ScalarValue::Null        => Bson::Null,
        ScalarValue::Bool(b)     => Bson::Boolean(*b),
        ScalarValue::Int32(i)    => Bson::Int32(*i),
        ScalarValue::Int64(i)    => Bson::Int64(*i),
        ScalarValue::Float64(f)  => Bson::Double(*f),
        ScalarValue::Str(s)      => Bson::String(s.clone()),
        ScalarValue::Binary(b)   => Bson::Binary(bson::Binary {
            subtype: bson::spec::BinarySubtype::Generic,
            bytes: b.clone(),
        }),
        ScalarValue::Timestamp(t) => {
            let dt = chrono::DateTime::parse_from_rfc3339(t)
                .expect("valid ISO 8601");
            Bson::DateTime(bson::DateTime::from_chrono(dt.with_timezone(&chrono::Utc)))
        }
    }
}

fn regex_escape(s: &str) -> String {
    regex::escape(s)
}
```

**Example.** Same domain filter:

```rust
Filter::and([
    Filter::eq("region", "us"),
    Filter::gte("created_at", Timestamp("2024-01-01T00:00:00Z".into())),
    Filter::or([
        Filter::eq("status", "active"),
        Filter::eq("status", "pending"),
    ]),
])
```

MongoDB backend produces:

```json
{
  "$and": [
    { "region": { "$eq": "us" } },
    { "created_at": { "$gte": ISODate("2024-01-01T00:00:00Z") } },
    { "$or": [
        { "status": { "$eq": "active" } },
        { "status": { "$eq": "pending" } }
    ]}
  ]
}
```

### PoloDB (default in-memory backend)

PoloDB uses the same BSON query syntax as MongoDB. The default backend reuses
the MongoDB filter translator and calls `polodb_core::Collection::find`:

```rust
// crates/wasi-jsondb/src/host/default_impl.rs

impl WasiJsonDbCtx for JsonDbDefault {
    fn query_collection(
        &self, name: &str, filter: Option<&FilterTree>, options: &QueryOpts,
    ) -> FutureResult<QueryResult> {
        let db = self.db.clone();
        async move {
            let collection = db.collection::<bson::Document>(name);
            let bson_filter = filter
                .map(mongodb_filter::to_bson)
                .unwrap_or_else(|| doc! {});
            let results: Vec<_> = collection.find(bson_filter)?.collect();
            Ok(to_query_result(results, options))
        }.boxed()
    }
}
```

---

## Data flow summary

```
Domain crate              SDK trait              Guest module            WIT boundary         Host backend
(platform-agnostic)       (omnia-sdk)            (wasi-jsondb)    (component ABI)      (PoloDB/Azure/Mongo)

Filter::and([             DocumentStore          store::query()          store.query           WasiJsonDbCtx
  Filter::eq("a","b"),      ::query(provider,      convert::              filter resource        ::query_collection()
  Filter::gt("c", 5),        store, options)        to_wit_filter()       static ctors             |
])                            |                     to_wit_value()        handle mgmt        FilterTree
                              |                     to_wit_op()           <---->                  |
                         cfg(wasm32):               to_wit_sort()                          +-----+------+
                           delegates to                                                    |            |
                           guest module                                                 to_odata()   to_bson()
                         cfg(not(wasm32)):                                                  |            |
                           abstract (MockProvider)                                   OData string  BSON Document
```

---

## Future work

### Optimistic concurrency

All three backends support conditional writes with version tokens:

- **Azure Table Storage**: Native ETags via `ETag` response header and `If-Match`
  request header. Unconditional upsert uses no `If-Match`; conditional update
  sends `If-Match: <etag>` and gets `412 Precondition Failed` on conflict.
- **MongoDB**: No native etags, but a managed `_etag` field in each document can
  serve the same purpose. Conditional writes filter on `{ _id: id, _etag: etag }`;
  `matched_count == 0` indicates conflict.
- **PoloDB**: Same field-based approach as MongoDB.

A future version could add:

- An `etag: option<string>` field to the `document` WIT record.
- A `conflict` variant to the `error` type.
- Separate SDK methods (`get_versioned`, `put_if_match`) or an optional etag parameter
  on `put`, so the simple path (unconditional writes) stays clean and the concurrency-aware
  path is opt-in.

The key design challenge is ergonomics: the guest must thread the etag from a previous
read to a subsequent write. Keeping the simple and versioned paths separate in the SDK
avoids polluting the common case.

### Field projection (`select`)

All three backends support returning a subset of fields:

- **Azure Table Storage**: `$select=field1,field2` query parameter.
- **MongoDB**: Projection `{ field1: 1, field2: 1 }`.
- **PoloDB**: Same as MongoDB.

A `select: list<string>` field on `query-options` could be added later. The main
concern is that partial documents make guest-side deserialization fragile -- the guest
would need to handle missing fields (`Option` on every field) or use a different
struct for projected queries. Deferred until there is a concrete need.

### Continuation token semantics

Continuation tokens are opaque strings whose internal representation is backend-dependent:

- **Azure Table Storage**: Encodes `NextPartitionKey` + `NextRowKey` from response headers.
- **MongoDB**: Encodes the last document's sort key for keyset pagination (preferred
  over cursor IDs which have server-side timeouts).
- **PoloDB**: Same keyset approach as MongoDB.

The guest never inspects or constructs tokens -- it passes them through from one
`query-result` to the next `query-options`. `None` on input starts from the beginning;
`None` on output means no more pages.
