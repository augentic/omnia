//! In-memory `DocumentStore` over the runtime's default docstore backend.

use std::future::Future;

use anyhow::Result;
use omnia_guest::DocumentStore;
use omnia_guest::document_store::{
    ComparisonOp, Document, Filter, QueryOptions, QueryResult, ScalarValue,
};
use omnia_wasi_docstore::{DocStoreDefault, FilterTree, QueryOpts, WasiDocStoreCtx};

// The host-side twins of the guest domain types.
mod host {
    pub use omnia_wasi_docstore::{ComparisonOp, Document, ScalarValue, SortField};
}

/// In-memory documents with the production default's filter, sort, and
/// pagination semantics; clones share one store.
///
/// Wraps `omnia_wasi_docstore::DocStoreDefault`, translating the guest's
/// `Filter`/`QueryOptions` into the host's `FilterTree`/`QueryOpts`, so a
/// query behaves exactly as it would through the runtime's in-memory
/// backend.
///
/// ```
/// use omnia_guest::DocumentStore as _;
/// use omnia_guest::document_store::{Document, Filter, QueryOptions};
/// use omnia_test::guest::MemoryDocs;
///
/// # tokio::runtime::Runtime::new().unwrap().block_on(async {
/// let docs = MemoryDocs::default();
/// for (id, age) in [("a", 17), ("b", 42)] {
///     let doc = Document {
///         id: id.into(),
///         data: format!(r#"{{"age":{age}}}"#).into_bytes(),
///     };
///     docs.put("people", &doc).await.unwrap();
/// }
/// let options = QueryOptions {
///     filter: Some(Filter::gte("age", 18)),
///     ..QueryOptions::default()
/// };
/// let adults = docs.query("people", options).await.unwrap();
/// assert_eq!(adults.documents[0].id, "b");
/// # });
/// ```
#[derive(Clone, Debug, Default)]
pub struct MemoryDocs {
    store: DocStoreDefault,
}

impl MemoryDocs {
    /// The wrapped host backend, for scenarios that also run the runtime.
    #[must_use]
    pub const fn backend(&self) -> &DocStoreDefault {
        &self.store
    }

    /// Every document in `store`, following continuations to the end.
    ///
    /// # Errors
    ///
    /// Returns the backend's error.
    pub async fn documents(&self, store: &str) -> Result<Vec<Document>> {
        let mut all = Vec::new();
        let mut continuation = None;
        loop {
            let page = self
                .store
                .query(
                    store.to_owned(),
                    None,
                    QueryOpts {
                        continuation,
                        ..QueryOpts::default()
                    },
                )
                .await?;
            all.extend(page.documents.into_iter().map(from_host));
            continuation = page.continuation;
            if continuation.is_none() {
                return Ok(all);
            }
        }
    }
}

impl DocumentStore for MemoryDocs {
    fn get(&self, store: &str, id: &str) -> impl Future<Output = Result<Option<Document>>> + Send {
        let pending = self.store.get(store.to_owned(), id.to_owned());
        async move { Ok(pending.await?.map(from_host)) }
    }

    fn insert(&self, store: &str, doc: &Document) -> impl Future<Output = Result<()>> + Send {
        self.store.insert(store.to_owned(), to_host(doc))
    }

    fn put(&self, store: &str, doc: &Document) -> impl Future<Output = Result<()>> + Send {
        self.store.put(store.to_owned(), to_host(doc))
    }

    fn delete(&self, store: &str, id: &str) -> impl Future<Output = Result<bool>> + Send {
        self.store.delete(store.to_owned(), id.to_owned())
    }

    fn query(
        &self, store: &str, options: QueryOptions,
    ) -> impl Future<Output = Result<QueryResult>> + Send {
        let filter = options.filter.map(filter_tree);
        let opts = QueryOpts {
            order_by: options
                .order_by
                .into_iter()
                .map(|sort| host::SortField {
                    field: sort.field,
                    descending: sort.descending,
                })
                .collect(),
            limit: options.limit,
            offset: options.offset,
            continuation: options.continuation,
        };
        let pending = self.store.query(store.to_owned(), filter, opts);
        async move {
            let result = pending.await?;
            Ok(QueryResult {
                documents: result.documents.into_iter().map(from_host).collect(),
                continuation: result.continuation,
            })
        }
    }
}

fn to_host(doc: &Document) -> host::Document {
    host::Document {
        id: doc.id.clone(),
        data: doc.data.clone(),
    }
}

fn from_host(doc: host::Document) -> Document {
    Document {
        id: doc.id,
        data: doc.data,
    }
}

fn scalar(value: ScalarValue) -> host::ScalarValue {
    match value {
        ScalarValue::Null => host::ScalarValue::Null,
        ScalarValue::Bool(b) => host::ScalarValue::Boolean(b),
        ScalarValue::Int32(i) => host::ScalarValue::Int32(i),
        ScalarValue::Int64(i) => host::ScalarValue::Int64(i),
        ScalarValue::Float64(f) => host::ScalarValue::Float64(f),
        ScalarValue::Str(s) => host::ScalarValue::Str(s),
        ScalarValue::Binary(b) => host::ScalarValue::Binary(b),
        ScalarValue::Timestamp(t) => host::ScalarValue::Timestamp(t),
    }
}

const fn op(op: ComparisonOp) -> host::ComparisonOp {
    match op {
        ComparisonOp::Eq => host::ComparisonOp::Eq,
        ComparisonOp::Ne => host::ComparisonOp::Ne,
        ComparisonOp::Gt => host::ComparisonOp::Gt,
        ComparisonOp::Gte => host::ComparisonOp::Gte,
        ComparisonOp::Lt => host::ComparisonOp::Lt,
        ComparisonOp::Lte => host::ComparisonOp::Lte,
    }
}

fn filter_tree(filter: Filter) -> FilterTree {
    match filter {
        Filter::Compare {
            field,
            op: cmp,
            value,
        } => FilterTree::Compare {
            field,
            op: op(cmp),
            value: scalar(value),
        },
        Filter::InList { field, values } => FilterTree::InList {
            field,
            values: values.into_iter().map(scalar).collect(),
        },
        Filter::NotInList { field, values } => FilterTree::NotInList {
            field,
            values: values.into_iter().map(scalar).collect(),
        },
        Filter::IsNull(field) => FilterTree::IsNull(field),
        Filter::IsNotNull(field) => FilterTree::IsNotNull(field),
        Filter::Contains { field, pattern } => FilterTree::Contains { field, pattern },
        Filter::StartsWith { field, pattern } => FilterTree::StartsWith { field, pattern },
        Filter::EndsWith { field, pattern } => FilterTree::EndsWith { field, pattern },
        Filter::And(children) => FilterTree::And(children.into_iter().map(filter_tree).collect()),
        Filter::Or(children) => FilterTree::Or(children.into_iter().map(filter_tree).collect()),
        Filter::Not(inner) => FilterTree::Not(Box::new(filter_tree(*inner))),
    }
}
