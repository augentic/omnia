//! JSON document store capability.

#[cfg(all(target_arch = "wasm32", target_os = "wasi"))]
mod convert;

use std::future::Future;

use serde::de::DeserializeOwned;

use crate::capability::{Page, Result};
use crate::docstore::{Document, Exists, Query};

/// Store and query JSON documents in a deployment's collections.
///
/// A query's filters and paging are defined by
/// [`Filter::matches`](crate::docstore::Filter::matches) and
/// [`SortField::compare`](crate::docstore::SortField::compare). Typed reads
/// of a document body are [`DocumentDbExt`].
///
/// Every provider serves the portable subset of a query:
///
/// - [`Filter::Compare`](crate::docstore::Filter::Compare) with a value other
///   than `null`, [`Filter::InList`](crate::docstore::Filter::InList), and
///   [`Filter::NotInList`](crate::docstore::Filter::NotInList), on a
///   top-level field;
/// - [`Filter::And`](crate::docstore::Filter::And),
///   [`Filter::Or`](crate::docstore::Filter::Or), and
///   [`Filter::Not`](crate::docstore::Filter::Not);
/// - a `limit`, with the [`Page::next`] cursor.
///
/// A zero `limit`, or a filter
/// [`Filter::check`](crate::docstore::Filter::check) refuses, fails with
/// [`ErrorKind::Invalid`](crate::capability::ErrorKind::Invalid). A provider
/// serves every other feature as defined, or fails the query with
/// [`ErrorKind::Unsupported`](crate::capability::ErrorKind::Unsupported), and
/// never ignores one. The
/// [document store reference](https://github.com/augentic/omnia/blob/main/docs/reference/docstore.md#query-contract)
/// lists the portable names and the optional features.
#[omnia_guest_macros::capability]
pub trait DocumentDb: Send + Sync {
    /// Fetch the document `id` from `collection`, or `None` when it is absent.
    fn get(
        &self, collection: &str, id: &str,
    ) -> impl Future<Output = Result<Option<Document>>> + Send;

    /// Insert `document` into `collection`, unless its id is already stored.
    ///
    /// A stored id answers `Ok(Err(Exists))` and leaves the stored document
    /// untouched.
    fn insert(
        &self, collection: &str, document: &Document,
    ) -> impl Future<Output = Result<Result<(), Exists>>> + Send;

    /// Store `document` in `collection`, replacing any document with its id.
    fn put(&self, collection: &str, document: &Document)
    -> impl Future<Output = Result<()>> + Send;

    /// Delete the document `id` from `collection`, answering whether one was removed.
    fn delete(&self, collection: &str, id: &str) -> impl Future<Output = Result<bool>> + Send;

    /// One page of the documents in `collection` that `query` selects.
    fn query(
        &self, collection: &str, query: &Query,
    ) -> impl Future<Output = Result<Page<Document>>> + Send;
}

/// Typed reads over any [`DocumentDb`].
///
/// Blanket-implemented for every [`DocumentDb`]; never implemented by hand.
pub trait DocumentDbExt: DocumentDb {
    /// Fetch the document `id` and decode its JSON body as `T`.
    ///
    /// A body that is not the JSON of a `T` fails with
    /// [`ErrorKind::Invalid`](crate::capability::ErrorKind::Invalid).
    fn get_as<T: DeserializeOwned>(
        &self, collection: &str, id: &str,
    ) -> impl Future<Output = Result<Option<T>>> + Send {
        async move { self.get(collection, id).await?.map(|document| document.decode()).transpose() }
    }
}

impl<P: DocumentDb + ?Sized> DocumentDbExt for P {}

#[cfg(all(target_arch = "wasm32", target_os = "wasi"))]
impl DocumentDb for crate::WasiProvider {
    async fn get(&self, collection: &str, id: &str) -> Result<Option<Document>> {
        let document =
            omnia_wasi_docstore::store::get(collection.to_owned(), id.to_owned()).await?;
        Ok(document.map(Into::into))
    }

    async fn insert(&self, collection: &str, document: &Document) -> Result<Result<(), Exists>> {
        let inserted =
            omnia_wasi_docstore::store::insert(collection.to_owned(), document.into()).await?;
        Ok(inserted.map_err(|_already_exists| Exists))
    }

    async fn put(&self, collection: &str, document: &Document) -> Result<()> {
        Ok(omnia_wasi_docstore::store::put(collection.to_owned(), document.into()).await?)
    }

    async fn delete(&self, collection: &str, id: &str) -> Result<bool> {
        Ok(omnia_wasi_docstore::store::delete(collection.to_owned(), id.to_owned()).await?)
    }

    async fn query(&self, collection: &str, query: &Query) -> Result<Page<Document>> {
        if let Some(filter) = &query.filter {
            filter.check()?;
        }
        let page = omnia_wasi_docstore::store::query(collection.to_owned(), query.into()).await?;
        Ok(page.into())
    }
}
