//! Conversions between the docstore value types and the `omnia:docstore` bindings.

use omnia_wasi_docstore::types;

use crate::docstore::{
    ComparisonOp, Document, Filter, QueryOptions, QueryResult, ScalarValue, SortField,
};

impl From<Filter> for types::Filter {
    fn from(filter: Filter) -> Self {
        match filter {
            Filter::Compare { field, op, value } => Self::compare(&field, op.into(), &value.into()),
            Filter::InList { field, values } => {
                Self::in_list(&field, &values.into_iter().map(Into::into).collect::<Vec<_>>())
            }
            Filter::NotInList { field, values } => {
                Self::not_in_list(&field, &values.into_iter().map(Into::into).collect::<Vec<_>>())
            }
            Filter::IsNull(field) => Self::is_null(&field),
            Filter::IsNotNull(field) => Self::is_not_null(&field),
            Filter::Contains { field, pattern } => Self::contains(&field, &pattern),
            Filter::StartsWith { field, pattern } => Self::starts_with(&field, &pattern),
            Filter::EndsWith { field, pattern } => Self::ends_with(&field, &pattern),
            Filter::And(children) => Self::and(children.into_iter().map(Into::into).collect()),
            Filter::Or(children) => Self::or(children.into_iter().map(Into::into).collect()),
            Filter::Not(inner) => Self::not((*inner).into()),
        }
    }
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

impl From<SortField> for types::SortField {
    fn from(sort: SortField) -> Self {
        Self {
            field: sort.field,
            descending: sort.descending,
        }
    }
}

impl From<QueryOptions> for types::QueryOptions {
    fn from(options: QueryOptions) -> Self {
        Self {
            filter: options.filter.map(Into::into),
            order_by: options.order_by.into_iter().map(Into::into).collect(),
            limit: options.limit,
            offset: options.offset,
            continuation: options.continuation,
        }
    }
}

impl From<&Document> for types::Document {
    fn from(document: &Document) -> Self {
        Self {
            id: document.id.clone(),
            data: document.data.clone(),
        }
    }
}

impl From<types::Document> for Document {
    fn from(document: types::Document) -> Self {
        Self {
            id: document.id,
            data: document.data,
        }
    }
}

impl From<types::QueryResult> for QueryResult {
    fn from(result: types::QueryResult) -> Self {
        Self {
            documents: result.documents.into_iter().map(Into::into).collect(),
            continuation: result.continuation,
        }
    }
}
