//! [`DocumentDb`] over `omnia:docstore`.

use bytes::Bytes;
use omnia_wasi_docstore::{store, types};

use crate::capability::{Error, ErrorKind, Page, Result};
use crate::docstore::{ComparisonOp, Document, Exists, Filter, Query, ScalarValue, SortField};
use crate::{DocumentDb, WasiProvider};

impl DocumentDb for WasiProvider {
    async fn get(&self, collection: &str, id: &str) -> Result<Option<Document>> {
        let document = store::get(collection.to_owned(), id.to_owned()).await?;
        Ok(document.map(Into::into))
    }

    async fn insert(&self, collection: &str, document: &Document) -> Result<Result<(), Exists>> {
        let inserted = store::insert(collection.to_owned(), document.into()).await?;
        Ok(inserted.map_err(|_already_exists| Exists))
    }

    async fn put(&self, collection: &str, document: &Document) -> Result<()> {
        Ok(store::put(collection.to_owned(), document.into()).await?)
    }

    async fn delete(&self, collection: &str, id: &str) -> Result<bool> {
        Ok(store::delete(collection.to_owned(), id.to_owned()).await?)
    }

    async fn query(&self, collection: &str, query: &Query) -> Result<Page<Document>> {
        if let Some(filter) = &query.filter {
            filter.check()?;
        }
        let page = store::query(collection.to_owned(), query.into()).await?;
        Ok(page.into())
    }
}

impl From<types::Error> for Error {
    fn from(err: types::Error) -> Self {
        let (kind, message) = match err {
            types::Error::NotFound(message) => (ErrorKind::NotFound, message),
            types::Error::Denied(message) => (ErrorKind::Denied, message),
            types::Error::Invalid(message) => (ErrorKind::Invalid, message),
            types::Error::Unsupported(message) => (ErrorKind::Unsupported, message),
            types::Error::Unavailable(message) => (ErrorKind::Unavailable, message),
            types::Error::Other(message) => (ErrorKind::Other, message),
        };
        Self::new(kind, message)
    }
}

impl From<&Filter> for types::Filter {
    fn from(filter: &Filter) -> Self {
        match filter {
            Filter::Compare { field, op, value } => {
                Self::compare(field, (*op).into(), &value.clone().into())
            }
            Filter::InList { field, values } => Self::in_list(field, &scalars(values)),
            Filter::NotInList { field, values } => Self::not_in_list(field, &scalars(values)),
            Filter::IsNull(field) => Self::is_null(field),
            Filter::IsNotNull(field) => Self::is_not_null(field),
            Filter::Contains { field, pattern } => Self::contains(field, pattern),
            Filter::StartsWith { field, pattern } => Self::starts_with(field, pattern),
            Filter::EndsWith { field, pattern } => Self::ends_with(field, pattern),
            Filter::And(children) => Self::and(children.iter().map(Into::into).collect()),
            Filter::Or(children) => Self::or(children.iter().map(Into::into).collect()),
            Filter::Not(inner) => Self::not((&**inner).into()),
        }
    }
}

fn scalars(values: &[ScalarValue]) -> Vec<types::ScalarValue> {
    values.iter().cloned().map(Into::into).collect()
}

impl From<ComparisonOp> for types::ComparisonOp {
    fn from(op: ComparisonOp) -> Self {
        match op {
            ComparisonOp::Eq => Self::Eq,
            ComparisonOp::Ne => Self::Ne,
            ComparisonOp::Gt => Self::Gt,
            ComparisonOp::Gte => Self::Gte,
            ComparisonOp::Lt => Self::Lt,
            ComparisonOp::Lte => Self::Lte,
        }
    }
}

impl From<ScalarValue> for types::ScalarValue {
    fn from(value: ScalarValue) -> Self {
        match value {
            ScalarValue::Null => Self::Null,
            ScalarValue::Bool(b) => Self::Boolean(b),
            ScalarValue::Int32(i) => Self::Int32(i),
            ScalarValue::Int64(i) => Self::Int64(i),
            ScalarValue::Float64(f) => Self::Float64(f),
            ScalarValue::Str(s) => Self::Str(s),
            ScalarValue::Binary(b) => Self::Binary(b),
            ScalarValue::Timestamp(t) => Self::Timestamp(t),
        }
    }
}

impl From<&SortField> for types::SortField {
    fn from(sort: &SortField) -> Self {
        Self {
            field: sort.field.clone(),
            descending: sort.descending,
        }
    }
}

impl From<&Query> for types::QueryOptions {
    fn from(query: &Query) -> Self {
        Self {
            filter: query.filter.as_ref().map(Into::into),
            order_by: query.order_by.iter().map(Into::into).collect(),
            limit: query.limit,
            offset: query.offset,
            continuation: query.after.clone(),
        }
    }
}

impl From<&Document> for types::Document {
    fn from(document: &Document) -> Self {
        Self {
            id: document.id.clone(),
            data: document.data.to_vec(),
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

impl From<types::QueryResult> for Page<Document> {
    fn from(result: types::QueryResult) -> Self {
        Self::new(result.documents.into_iter().map(Into::into).collect(), result.continuation)
    }
}
