//! Target-neutral domain types and service contract for `omnia:docstore`.

use std::ops::Not;

/// Scalar values for filter comparisons.
#[derive(Debug, Clone, PartialEq)]
pub enum ScalarValue {
    /// Null literal.
    Null,
    /// Boolean.
    Bool(bool),
    /// 32-bit integer.
    Int32(i32),
    /// 64-bit integer.
    Int64(i64),
    /// Floating point.
    Float64(f64),
    /// UTF-8 string.
    Str(String),
    /// Opaque bytes.
    Binary(Vec<u8>),
    /// ISO-8601 timestamp string for comparisons.
    Timestamp(String),
}

/// Comparison operators for filters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComparisonOp {
    /// Equal.
    Eq,
    /// Not equal.
    Ne,
    /// Greater than.
    Gt,
    /// Greater than or equal.
    Gte,
    /// Less than.
    Lt,
    /// Less than or equal.
    Lte,
}

/// A filter expression tree.
#[derive(Debug, Clone)]
pub enum Filter {
    /// Compare `field` to `value` using `op`.
    Compare {
        /// Field path.
        field: String,
        /// Comparison operator.
        op: ComparisonOp,
        /// Right-hand value.
        value: ScalarValue,
    },
    /// Field value is one of the given values.
    InList {
        /// Field path.
        field: String,
        /// Allowed values.
        values: Vec<ScalarValue>,
    },
    /// Field value is not in the given set.
    NotInList {
        /// Field path.
        field: String,
        /// Excluded values.
        values: Vec<ScalarValue>,
    },
    /// Field is null or missing.
    IsNull(String),
    /// Field exists and is not null.
    IsNotNull(String),
    /// String contains pattern (backend-defined semantics).
    Contains {
        /// Field path.
        field: String,
        /// Substring pattern.
        pattern: String,
    },
    /// String starts with pattern.
    StartsWith {
        /// Field path.
        field: String,
        /// Prefix pattern.
        pattern: String,
    },
    /// String ends with pattern.
    EndsWith {
        /// Field path.
        field: String,
        /// Suffix pattern.
        pattern: String,
    },
    /// Logical AND of child filters.
    And(Vec<Self>),
    /// Logical OR of child filters.
    Or(Vec<Self>),
    /// Logical NOT.
    Not(Box<Self>),
}

/// Sort field for queries.
#[derive(Debug, Clone, Default)]
pub struct SortField {
    /// Field path.
    pub field: String,
    /// When `true`, sort descending.
    pub descending: bool,
}

/// Options for listing or searching documents.
#[derive(Debug, Clone, Default)]
pub struct QueryOptions {
    /// Optional filter tree.
    pub filter: Option<Filter>,
    /// Sort order (first key wins, then next, etc.).
    pub order_by: Vec<SortField>,
    /// Maximum documents to return.
    pub limit: Option<u32>,
    /// Skip this many documents after filter/sort (offset pagination).
    pub offset: Option<u32>,
    /// Opaque continuation token from a previous page.
    pub continuation: Option<String>,
}

/// Stored document: identifier plus JSON body bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Document {
    /// Primary key string.
    pub id: String,
    /// JSON payload.
    pub data: Vec<u8>,
}

/// Result of a query with optional next-page token.
#[derive(Debug, Clone, Default)]
pub struct QueryResult {
    /// Matching documents.
    pub documents: Vec<Document>,
    /// Continuation token for the next page, if any.
    pub continuation: Option<String>,
}

impl From<&str> for ScalarValue {
    fn from(s: &str) -> Self {
        Self::Str(s.to_string())
    }
}

impl From<String> for ScalarValue {
    fn from(s: String) -> Self {
        Self::Str(s)
    }
}

impl From<i32> for ScalarValue {
    fn from(v: i32) -> Self {
        Self::Int32(v)
    }
}

impl From<i64> for ScalarValue {
    fn from(v: i64) -> Self {
        Self::Int64(v)
    }
}

impl From<f64> for ScalarValue {
    fn from(v: f64) -> Self {
        Self::Float64(v)
    }
}

impl From<bool> for ScalarValue {
    fn from(v: bool) -> Self {
        Self::Bool(v)
    }
}

/// Newtype for timestamp strings so `Filter::gte("ts", Timestamp(...))` is explicit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Timestamp(pub String);

impl From<Timestamp> for ScalarValue {
    fn from(t: Timestamp) -> Self {
        Self::Timestamp(t.0)
    }
}

impl Filter {
    /// Compare `field` to `val` with `op`.
    #[must_use]
    pub fn cmp(field: &str, op: ComparisonOp, val: impl Into<ScalarValue>) -> Self {
        Self::Compare {
            field: field.to_string(),
            op,
            value: val.into(),
        }
    }

    /// Equality comparison.
    #[must_use]
    pub fn eq(field: &str, val: impl Into<ScalarValue>) -> Self {
        Self::cmp(field, ComparisonOp::Eq, val)
    }

    /// Inequality comparison.
    #[must_use]
    pub fn ne(field: &str, val: impl Into<ScalarValue>) -> Self {
        Self::cmp(field, ComparisonOp::Ne, val)
    }

    /// Greater than.
    #[must_use]
    pub fn gt(field: &str, val: impl Into<ScalarValue>) -> Self {
        Self::cmp(field, ComparisonOp::Gt, val)
    }

    /// Greater than or equal.
    #[must_use]
    pub fn gte(field: &str, val: impl Into<ScalarValue>) -> Self {
        Self::cmp(field, ComparisonOp::Gte, val)
    }

    /// Less than.
    #[must_use]
    pub fn lt(field: &str, val: impl Into<ScalarValue>) -> Self {
        Self::cmp(field, ComparisonOp::Lt, val)
    }

    /// Less than or equal.
    #[must_use]
    pub fn lte(field: &str, val: impl Into<ScalarValue>) -> Self {
        Self::cmp(field, ComparisonOp::Lte, val)
    }

    /// Field value is in the given set.
    #[must_use]
    pub fn in_list(field: &str, vals: impl IntoIterator<Item = impl Into<ScalarValue>>) -> Self {
        Self::InList {
            field: field.to_string(),
            values: vals.into_iter().map(Into::into).collect(),
        }
    }

    /// Field value is not in the given set.
    #[must_use]
    pub fn not_in_list(
        field: &str, vals: impl IntoIterator<Item = impl Into<ScalarValue>>,
    ) -> Self {
        Self::NotInList {
            field: field.to_string(),
            values: vals.into_iter().map(Into::into).collect(),
        }
    }

    /// Field is null or missing.
    #[must_use]
    pub fn is_null(field: &str) -> Self {
        Self::IsNull(field.to_string())
    }

    /// Field exists and is not null.
    #[must_use]
    pub fn is_not_null(field: &str) -> Self {
        Self::IsNotNull(field.to_string())
    }

    /// String contains pattern.
    #[must_use]
    pub fn contains(field: &str, pattern: &str) -> Self {
        Self::Contains {
            field: field.to_string(),
            pattern: pattern.to_string(),
        }
    }

    /// String starts with pattern.
    #[must_use]
    pub fn starts_with(field: &str, pattern: &str) -> Self {
        Self::StartsWith {
            field: field.to_string(),
            pattern: pattern.to_string(),
        }
    }

    /// String ends with pattern.
    #[must_use]
    pub fn ends_with(field: &str, pattern: &str) -> Self {
        Self::EndsWith {
            field: field.to_string(),
            pattern: pattern.to_string(),
        }
    }

    /// Logical AND.
    #[must_use]
    pub fn and(filters: impl IntoIterator<Item = Self>) -> Self {
        Self::And(filters.into_iter().collect())
    }

    /// Logical OR.
    #[must_use]
    pub fn or(filters: impl IntoIterator<Item = Self>) -> Self {
        Self::Or(filters.into_iter().collect())
    }

    /// Logical NOT.
    #[must_use]
    pub fn negate(inner: Self) -> Self {
        !inner
    }

    /// Maximum nesting depth of the filter tree.
    #[must_use]
    pub fn depth(&self) -> usize {
        match self {
            Self::And(children) | Self::Or(children) => {
                1 + children.iter().map(Self::depth).max().unwrap_or(0)
            }
            Self::Not(inner) => 1 + inner.depth(),
            _ => 1,
        }
    }

    /// Restrict `field` to a calendar date (UTC day) using range on timestamp strings.
    ///
    /// # Errors
    ///
    /// Returns an error if `iso_date` is not a valid `YYYY-MM-DD` date or is the
    /// maximum representable date (no next day).
    pub fn on_date(field: &str, iso_date: &str) -> anyhow::Result<Self> {
        let next = next_iso_date(iso_date)?;
        let start = format!("{iso_date}T00:00:00Z");
        let end = format!("{next}T00:00:00Z");
        Ok(Self::And(vec![Self::gte(field, Timestamp(start)), Self::lt(field, Timestamp(end))]))
    }
}

impl Not for Filter {
    type Output = Self;

    fn not(self) -> Self::Output {
        Self::Not(Box::new(self))
    }
}

fn next_iso_date(iso_date: &str) -> anyhow::Result<String> {
    use chrono::NaiveDate;
    let date = NaiveDate::parse_from_str(iso_date, "%Y-%m-%d")
        .map_err(|_e| anyhow::anyhow!("invalid ISO date: {iso_date:?}"))?;
    let next = date
        .succ_opt()
        .ok_or_else(|| anyhow::anyhow!("cannot compute next day for date: {iso_date}"))?;
    Ok(next.format("%Y-%m-%d").to_string())
}

#[cfg(not(target_arch = "wasm32"))]
mod filter;

#[cfg(not(target_arch = "wasm32"))]
mod host {
    use std::collections::btree_map::Entry;
    use std::collections::{BTreeMap, HashMap};
    use std::fmt::Debug;
    use std::sync::{Arc, RwLock};

    use anyhow::{Context, Result};
    use futures::FutureExt as _;
    use serde_json::Value;
    use tracing::instrument;

    use super::{Document, QueryOptions, QueryResult, filter};
    use crate::{Connect, FutureResult, NoOptions};

    const MAX_PAGE_SIZE: u64 = 1000;

    /// Service contract for `omnia:docstore` backends.
    pub trait DocStoreClient: Debug + Send + Sync + 'static {
        /// Point read by primary id.
        fn get(&self, collection: String, id: String) -> FutureResult<Option<Document>>;

        /// Insert if absent.
        fn insert(&self, collection: String, doc: Document) -> FutureResult<()>;

        /// Upsert by id.
        fn put(&self, collection: String, doc: Document) -> FutureResult<()>;

        /// Delete by id; `Ok(true)` if a document was removed.
        fn delete(&self, collection: String, id: String) -> FutureResult<bool>;

        /// Filtered listing with sort and pagination.
        fn query(&self, collection: String, options: QueryOptions) -> FutureResult<QueryResult>;
    }

    type Collections = HashMap<String, BTreeMap<String, Value>>;

    /// Default in-memory implementation of [`DocStoreClient`].
    #[derive(Clone, Default)]
    pub struct DocStoreDefault {
        collections: Arc<RwLock<Collections>>,
    }

    impl Debug for DocStoreDefault {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("DocStoreDefault").finish_non_exhaustive()
        }
    }

    impl Connect for DocStoreDefault {
        type ConnectOptions = NoOptions;

        #[instrument]
        async fn connect_with(options: Self::ConnectOptions) -> Result<Self> {
            let _ = options;
            tracing::debug!("initializing in-memory document store");
            Ok(Self::default())
        }
    }

    impl DocStoreDefault {
        fn read(&self) -> std::sync::RwLockReadGuard<'_, Collections> {
            self.collections.read().expect("docstore lock poisoned")
        }

        fn write(&self) -> std::sync::RwLockWriteGuard<'_, Collections> {
            self.collections.write().expect("docstore lock poisoned")
        }
    }

    impl DocStoreClient for DocStoreDefault {
        fn get(&self, collection: String, id: String) -> FutureResult<Option<Document>> {
            let store = self.clone();
            async move {
                let guard = store.read();
                guard
                    .get(&collection)
                    .and_then(|col| col.get(&id))
                    .map(|body| to_document(&id, body))
                    .transpose()
            }
            .boxed()
        }

        fn insert(&self, collection: String, doc: Document) -> FutureResult<()> {
            let store = self.clone();
            async move {
                let body = parse_body(&doc)?;
                let inserted = match store.write().entry(collection).or_default().entry(doc.id) {
                    Entry::Vacant(slot) => {
                        slot.insert(body);
                        true
                    }
                    Entry::Occupied(_) => false,
                };
                anyhow::ensure!(inserted, "document id already exists");
                Ok(())
            }
            .boxed()
        }

        fn put(&self, collection: String, doc: Document) -> FutureResult<()> {
            let store = self.clone();
            async move {
                let body = parse_body(&doc)?;
                store.write().entry(collection).or_default().insert(doc.id, body);
                Ok(())
            }
            .boxed()
        }

        fn delete(&self, collection: String, id: String) -> FutureResult<bool> {
            let store = self.clone();
            async move {
                Ok(store.write().get_mut(&collection).is_some_and(|col| col.remove(&id).is_some()))
            }
            .boxed()
        }

        fn query(&self, collection: String, options: QueryOptions) -> FutureResult<QueryResult> {
            let store = self.clone();
            async move {
                let limit = options.limit.map_or(MAX_PAGE_SIZE, u64::from).min(MAX_PAGE_SIZE);
                anyhow::ensure!(limit > 0, "query limit must be at least 1");
                let page_size = usize::try_from(limit).unwrap_or(usize::MAX);

                let skip = parse_continuation(options.continuation.as_deref())?
                    + u64::from(options.offset.unwrap_or(0));
                let skip = usize::try_from(skip).unwrap_or(usize::MAX);

                let mut rows: Vec<(String, Value)> = {
                    let guard = store.read();
                    guard.get(&collection).map_or_else(Vec::new, |col| {
                        col.iter()
                            .filter(|(_, body)| {
                                options.filter.as_ref().is_none_or(|f| filter::matches(f, body))
                            })
                            .map(|(id, body)| (id.clone(), body.clone()))
                            .collect()
                    })
                };

                if !options.order_by.is_empty() {
                    rows.sort_by(|a, b| {
                        filter::compare_documents(&a.1, &b.1, &options.order_by)
                            .then_with(|| a.0.cmp(&b.0))
                    });
                }

                let (page, continuation) = paginate(rows, skip, page_size);
                let documents = page
                    .iter()
                    .map(|(id, body)| to_document(id, body))
                    .collect::<Result<Vec<_>>>()?;

                Ok(QueryResult {
                    documents,
                    continuation,
                })
            }
            .boxed()
        }
    }

    fn paginate(
        rows: Vec<(String, Value)>, skip: usize, page_size: usize,
    ) -> (Vec<(String, Value)>, Option<String>) {
        let total = rows.len();
        let page: Vec<_> = rows.into_iter().skip(skip).take(page_size).collect();
        let next = skip.saturating_add(page.len());
        let continuation = (next < total).then(|| next.to_string());
        (page, continuation)
    }

    fn parse_continuation(continuation: Option<&str>) -> Result<u64> {
        continuation.map_or_else(
            || Ok(0),
            |s| s.parse().with_context(|| format!("invalid continuation token: {s:?}")),
        )
    }

    fn parse_body(doc: &Document) -> Result<Value> {
        serde_json::from_slice(&doc.data).context("invalid JSON in document body")
    }

    fn to_document(id: &str, body: &Value) -> Result<Document> {
        Ok(Document {
            id: id.to_owned(),
            data: serde_json::to_vec(body).context("serialize JSON body")?,
        })
    }

    #[cfg(test)]
    mod tests {
        use serde_json::json;

        use super::*;

        fn rows(ids: &[&str]) -> Vec<(String, Value)> {
            ids.iter().map(|id| ((*id).to_string(), json!({ "id": id }))).collect()
        }

        #[test]
        fn paginate_walks_all_pages() {
            let (page, cont) = paginate(rows(&["a", "b", "c", "d", "e"]), 0, 2);
            assert_eq!(page.len(), 2);
            assert_eq!(cont.as_deref(), Some("2"));

            let (page, cont) = paginate(rows(&["a", "b", "c", "d", "e"]), 2, 2);
            assert_eq!(page.len(), 2);
            assert_eq!(cont.as_deref(), Some("4"));

            let (page, cont) = paginate(rows(&["a", "b", "c", "d", "e"]), 4, 2);
            assert_eq!(page.len(), 1);
            assert_eq!(cont, None, "final partial page carries no continuation");
        }

        #[test]
        fn paginate_exact_boundary() {
            let (page, cont) = paginate(rows(&["a", "b"]), 0, 2);
            assert_eq!(page.len(), 2);
            assert_eq!(cont, None, "a full final page carries no continuation");
        }

        #[test]
        fn continuation_rejects_junk() {
            parse_continuation(Some("not-a-number")).expect_err("garbage token is rejected");
            assert_eq!(parse_continuation(None).expect("empty token"), 0);
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub use self::host::{DocStoreClient, DocStoreDefault};

#[cfg(test)]
mod tests {
    use super::*;

    fn bound(filter: &Filter, expected_op: ComparisonOp) -> &str {
        let Filter::Compare {
            field,
            op,
            value: ScalarValue::Timestamp(ts),
        } = filter
        else {
            panic!("expected timestamp comparison, got {filter:?}");
        };
        assert_eq!(field, "updated");
        assert_eq!(*op, expected_op);
        ts
    }

    #[test]
    fn on_date_day_range() {
        let Filter::And(children) = Filter::on_date("updated", "2026-03-19").unwrap() else {
            panic!("expected And");
        };
        assert_eq!(children.len(), 2);
        assert_eq!(bound(&children[0], ComparisonOp::Gte), "2026-03-19T00:00:00Z");
        assert_eq!(bound(&children[1], ComparisonOp::Lt), "2026-03-20T00:00:00Z");
    }

    #[test]
    fn on_date_month_end() {
        let Filter::And(children) = Filter::on_date("updated", "2026-12-31").unwrap() else {
            panic!("expected And");
        };
        assert_eq!(bound(&children[1], ComparisonOp::Lt), "2027-01-01T00:00:00Z");
    }

    #[test]
    fn on_date_invalid() {
        Filter::on_date("updated", "2026-13-01").unwrap_err();
        Filter::on_date("updated", "not-a-date").unwrap_err();
    }
}
