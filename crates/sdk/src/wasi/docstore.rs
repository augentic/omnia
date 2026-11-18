//! [`DocumentDb`] over `omnia:docstore`.

use bytes::Bytes;
use omnia_wasi_docstore::omnia::collection::{self, Collection};
use omnia_wasi_docstore::omnia::store;
use omnia_wasi_docstore::omnia::types::{self, Precondition};

use crate::docstore::groups::{Condition, Test, Value};
use crate::docstore::{Document, Exists, Query, SortField};
use crate::{DocumentDb, Error, Page, Result, WasiProvider};

impl DocumentDb for WasiProvider {
    async fn get(&self, collection: &str, id: &str) -> Result<Option<Document>> {
        let document = open(collection).await?.get(id.to_owned()).await?;
        Ok(document.map(Into::into))
    }

    async fn insert(&self, collection: &str, document: &Document) -> Result<Result<(), Exists>> {
        let (id, data) = (document.id.clone(), document.data.to_vec());
        let written = open(collection).await?.put(id, data, Some(Precondition::Absent)).await?;
        Ok(written.map(|_revision| ()).map_err(|_conflict| Exists))
    }

    async fn put(&self, collection: &str, document: &Document) -> Result<()> {
        let (id, data) = (document.id.clone(), document.data.to_vec());
        open(collection).await?.put(id, data, None).await??;
        Ok(())
    }

    // `exists` then `delete` is two calls, so a concurrent write can change
    // the answer between them
    async fn delete(&self, collection: &str, id: &str) -> Result<bool> {
        let collection = open(collection).await?;
        let existed = collection.exists(id.to_owned()).await?;
        collection.delete(id.to_owned(), None).await??;
        Ok(existed)
    }

    async fn query(&self, collection: &str, query: &Query) -> Result<Page<Document>> {
        if query.offset.is_some() {
            return Err(Error::server_error("unsupported", "a document query takes no `offset`"));
        }
        let filter = match &query.filter {
            Some(filter) => filter.groups()?,
            None => Vec::new(),
        };
        let filter = filter.into_iter().map(|group| group.into_iter().map(Into::into).collect());
        let order_by = query.order_by.iter().map(Into::into).collect();

        let page = open(collection)
            .await?
            .query(filter.collect(), order_by, query.after.clone(), query.limit)
            .await?;
        Ok(Page::new(page.documents.into_iter().map(Into::into).collect(), page.next))
    }
}

async fn open(name: &str) -> Result<Collection> {
    Ok(store::open(name.to_owned()).await?)
}

impl From<types::Error> for Error {
    fn from(err: types::Error) -> Self {
        match err {
            types::Error::NotFound(message) => Self::server_error("not-found", message),
            types::Error::Denied(message) => Self::server_error("denied", message),
            types::Error::Invalid(message) => Self::server_error("invalid", message),
            types::Error::Unsupported(message) => Self::server_error("unsupported", message),
            types::Error::Unavailable(message) => Self::bad_gateway("unavailable", message),
            types::Error::Other(message) => Self::server_error("other", message),
        }
    }
}

// a write with no precondition has none to fail
impl From<types::Conflict> for Error {
    fn from(_: types::Conflict) -> Self {
        Self::server_error("other", "the host reported a conflict on an unconditional write")
    }
}

impl From<Condition> for collection::Condition {
    fn from(condition: Condition) -> Self {
        Self {
            field: condition.field,
            test: condition.test.into(),
            negated: condition.negated,
        }
    }
}

impl From<Test> for collection::Test {
    fn from(test: Test) -> Self {
        match test {
            Test::Eq(value) => Self::Eq(value.into()),
            Test::Lt(value) => Self::Lt(value.into()),
            Test::Lte(value) => Self::Lte(value.into()),
            Test::Gt(value) => Self::Gt(value.into()),
            Test::Gte(value) => Self::Gte(value.into()),
            Test::In(values) => Self::In(values.into_iter().map(Into::into).collect()),
            Test::StartsWith(pattern) => Self::StartsWith(pattern),
            Test::EndsWith(pattern) => Self::EndsWith(pattern),
            Test::Contains(pattern) => Self::Contains(pattern),
            Test::Null => Self::Null,
        }
    }
}

impl From<Value> for collection::Value {
    fn from(value: Value) -> Self {
        match value {
            Value::Boolean(boolean) => Self::Boolean(boolean),
            Value::Integer(integer) => Self::Integer(integer),
            Value::Number(number) => Self::Number(number),
            Value::Text(text) => Self::Text(text),
        }
    }
}

impl From<&SortField> for collection::SortField {
    fn from(sort: &SortField) -> Self {
        Self {
            field: sort.field.clone(),
            descending: sort.descending,
        }
    }
}

impl From<types::Document> for Document {
    fn from(document: types::Document) -> Self {
        Self {
            id: document.id,
            data: Bytes::from(document.data),
        }
    }
}
