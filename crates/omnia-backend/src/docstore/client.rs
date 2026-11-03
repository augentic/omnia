//! Service contract for `omnia:docstore` backends.

use std::fmt::Debug;

use super::{Document, QueryOptions, QueryResult};
use crate::FutureResult;

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
