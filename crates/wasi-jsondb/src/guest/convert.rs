//! Convert between [`crate::document_store`] types and generated WIT types.

use crate::document_store as sdk;
use crate::guest::generated::wasi::jsondb::types::{
    ComparisonOp as WitComparisonOp, Document as WitDocument, Filter as WitFilterHandle,
    QueryOptions as WitQueryOptions, QueryResult as WitQueryResult, ScalarValue as WitScalarValue,
    SortField as WitSortField,
};

/// Map SDK filter to WIT filter resource constructors.
#[must_use]
pub(super) fn to_wit_filter(filter: sdk::Filter) -> WitFilterHandle {
    match filter {
        sdk::Filter::Compare { field, op, value } => {
            WitFilterHandle::compare(&field, to_wit_op(op), &to_wit_value(value))
        }
        sdk::Filter::InList { field, values } => {
            let vals: Vec<_> = values.into_iter().map(to_wit_value).collect();
            WitFilterHandle::in_list(&field, &vals)
        }
        sdk::Filter::NotInList { field, values } => {
            let vals: Vec<_> = values.into_iter().map(to_wit_value).collect();
            WitFilterHandle::not_in_list(&field, &vals)
        }
        sdk::Filter::IsNull(field) => WitFilterHandle::is_null(&field),
        sdk::Filter::IsNotNull(field) => WitFilterHandle::is_not_null(&field),
        sdk::Filter::Contains { field, pattern } => WitFilterHandle::contains(&field, &pattern),
        sdk::Filter::StartsWith { field, pattern } => {
            WitFilterHandle::starts_with(&field, &pattern)
        }
        sdk::Filter::EndsWith { field, pattern } => WitFilterHandle::ends_with(&field, &pattern),
        sdk::Filter::And(children) => {
            let wit_children: Vec<_> = children.into_iter().map(to_wit_filter).collect();
            WitFilterHandle::and(wit_children)
        }
        sdk::Filter::Or(children) => {
            let wit_children: Vec<_> = children.into_iter().map(to_wit_filter).collect();
            WitFilterHandle::or(wit_children)
        }
        sdk::Filter::Not(inner) => WitFilterHandle::not(to_wit_filter(*inner)),
    }
}

#[must_use]
pub(super) fn to_wit_op(op: sdk::ComparisonOp) -> WitComparisonOp {
    match op {
        sdk::ComparisonOp::Eq => WitComparisonOp::Eq,
        sdk::ComparisonOp::Ne => WitComparisonOp::Ne,
        sdk::ComparisonOp::Gt => WitComparisonOp::Gt,
        sdk::ComparisonOp::Gte => WitComparisonOp::Gte,
        sdk::ComparisonOp::Lt => WitComparisonOp::Lt,
        sdk::ComparisonOp::Lte => WitComparisonOp::Lte,
    }
}

#[must_use]
pub(super) fn to_wit_value(v: sdk::ScalarValue) -> WitScalarValue {
    match v {
        sdk::ScalarValue::Null => WitScalarValue::Null,
        sdk::ScalarValue::Bool(b) => WitScalarValue::Boolean(b),
        sdk::ScalarValue::Int32(i) => WitScalarValue::Int32(i),
        sdk::ScalarValue::Int64(i) => WitScalarValue::Int64(i),
        sdk::ScalarValue::Float64(f) => WitScalarValue::Float64(f),
        sdk::ScalarValue::Str(s) => WitScalarValue::Str(s),
        sdk::ScalarValue::Binary(b) => WitScalarValue::Binary(b),
        sdk::ScalarValue::Timestamp(t) => WitScalarValue::Timestamp(t),
    }
}

#[must_use]
pub(super) fn to_wit_sort(s: &sdk::SortField) -> WitSortField {
    WitSortField {
        field: s.field.clone(),
        descending: s.descending,
    }
}

#[must_use]
pub(super) fn to_wit_document(d: &sdk::Document) -> WitDocument {
    WitDocument {
        id: d.id.clone(),
        data: d.data.clone(),
    }
}

#[must_use]
pub(super) fn to_wit_query_options(o: sdk::QueryOptions) -> WitQueryOptions {
    WitQueryOptions {
        filter: o.filter.map(to_wit_filter),
        order_by: o.order_by.iter().map(to_wit_sort).collect(),
        limit: o.limit,
        offset: o.offset,
        continuation: o.continuation,
    }
}

#[must_use]
pub(super) fn from_wit_document(d: WitDocument) -> sdk::Document {
    sdk::Document {
        id: d.id,
        data: d.data,
    }
}

#[must_use]
pub(super) fn from_wit_query_result(r: WitQueryResult) -> sdk::QueryResult {
    sdk::QueryResult {
        documents: r.documents.into_iter().map(from_wit_document).collect(),
        continuation: r.continuation,
    }
}
