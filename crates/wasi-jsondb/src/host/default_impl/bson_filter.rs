//! Translate [`FilterTree`](crate::host::resource::FilterTree) to BSON for the default `PoloDB` backend.
//!
//! Type decisions here match how `bson::to_document(&serde_json::Value)` stores documents:
//! - JSON integers (positive/negative) → `Bson::Int64`, so `ScalarValue::Int32` is upcast
//! - JSON strings (including timestamps) → `Bson::String`, so `ScalarValue::Timestamp` stays as string
//! - Regex uses `{ "$regex": bson::Regex }` operator syntax (`PoloDB` ignores direct `RegularExpression` values)
//!
//! # `PoloDB` 5.1.4 bugs & workarounds
//!
//! The following bugs were observed in `polodb_core` 5.1.4 (<https://github.com/PoloDB/PoloDB>).
//! No upstream issue exists at time of writing; file one if these are still present in a newer release.
//!
//! | Bug | Workaround |
//! |-----|------------|
//! | `$ne` returns the **equal** document (behaves as `$eq`) | `{ "$not": { "$eq": … } }` |
//! | `$nin` returns the **matching** documents (behaves as `$in`) | `{ "$not": { "$in": … } }` |
//! | `Bson::RegularExpression` value is silently ignored in queries | `{ "$regex": bson::Regex { … } }` operator syntax |
//! | Regex anchors `^` / `$` are silently ignored | `StartsWith`/`EndsWith` degrade to `Contains` |

use polodb_core::bson::{self, Bson, Document, doc};

use crate::host::generated::wasi::jsondb::types::{ComparisonOp, ScalarValue};
use crate::host::resource::FilterTree;

/// Convert a host filter tree to a BSON query document.
#[must_use]
pub fn to_bson(tree: &FilterTree) -> Document {
    match tree {
        FilterTree::Compare { field, op, value } => compare_to_bson(field, *op, value),
        FilterTree::InList { field, values } => {
            let bson_vals: Vec<Bson> = values.iter().map(to_bson_value).collect();
            doc! { field: { "$in": bson_vals } }
        }
        FilterTree::NotInList { field, values } => {
            let bson_vals: Vec<Bson> = values.iter().map(to_bson_value).collect();
            doc! { field: { "$not": { "$in": bson_vals } } }
        }
        FilterTree::IsNull(field) => doc! { field: Bson::Null },
        FilterTree::IsNotNull(field) => {
            doc! { field: { "$not": { "$eq": Bson::Null } } }
        }
        FilterTree::Contains { field, pattern } => {
            doc! { field: { "$regex": to_regex(&regex_escape(pattern)) } }
        }
        // PoloDB ignores regex anchors ^ and $, so StartsWith/EndsWith
        // cannot be evaluated server-side. Return empty filter (matches all);
        // the caller post-filters with `post_filter_matches`.
        FilterTree::StartsWith { .. } | FilterTree::EndsWith { .. } => doc! {},
        FilterTree::And(children) => {
            let docs: Vec<Bson> = children.iter().map(|c| Bson::Document(to_bson(c))).collect();
            doc! { "$and": docs }
        }
        FilterTree::Or(children) => {
            let docs: Vec<Bson> = children.iter().map(|c| Bson::Document(to_bson(c))).collect();
            doc! { "$or": docs }
        }
        FilterTree::Not(inner) => negate_to_bson(inner),
    }
}

/// Emit a single comparison, routing `Ne` through `$not` to work around the `PoloDB` `$ne` bug.
fn compare_to_bson(field: &str, op: ComparisonOp, value: &ScalarValue) -> Document {
    let v = to_bson_value(value);
    if op == ComparisonOp::Ne {
        return doc! { field: { "$not": { "$eq": v } } };
    }
    let bson_op = match op {
        ComparisonOp::Eq => "$eq",
        ComparisonOp::Gt => "$gt",
        ComparisonOp::Gte => "$gte",
        ComparisonOp::Lt => "$lt",
        ComparisonOp::Lte => "$lte",
        ComparisonOp::Ne => unreachable!(),
    };
    doc! { field: { bson_op: v } }
}

/// Negate a filter tree into BSON using `$not` wrappers.
fn negate_to_bson(inner: &FilterTree) -> Document {
    match inner {
        FilterTree::Compare { field, op, value } => {
            let v = to_bson_value(value);
            let bson_op = match op {
                ComparisonOp::Eq => "$eq",
                ComparisonOp::Ne => "$ne",
                ComparisonOp::Gt => "$gt",
                ComparisonOp::Gte => "$gte",
                ComparisonOp::Lt => "$lt",
                ComparisonOp::Lte => "$lte",
            };
            doc! { field: { "$not": { bson_op: v } } }
        }
        FilterTree::InList { field, values } => {
            let bson_vals: Vec<Bson> = values.iter().map(to_bson_value).collect();
            doc! { field: { "$not": { "$in": bson_vals } } }
        }
        FilterTree::NotInList { field, values } => {
            let bson_vals: Vec<Bson> = values.iter().map(to_bson_value).collect();
            doc! { field: { "$in": bson_vals } }
        }
        FilterTree::IsNull(field) => {
            doc! { field: { "$not": { "$eq": Bson::Null } } }
        }
        FilterTree::IsNotNull(field) => doc! { field: Bson::Null },
        FilterTree::Contains { field, pattern } => {
            doc! { field: { "$not": { "$regex": to_regex(&regex_escape(pattern)) } } }
        }
        // Negated StartsWith/EndsWith: also deferred to post-filter.
        FilterTree::StartsWith { .. } | FilterTree::EndsWith { .. } => doc! {},
        FilterTree::And(children) => {
            let docs: Vec<Bson> =
                children.iter().map(|c| Bson::Document(negate_to_bson(c))).collect();
            doc! { "$or": docs }
        }
        FilterTree::Or(children) => {
            let docs: Vec<Bson> =
                children.iter().map(|c| Bson::Document(negate_to_bson(c))).collect();
            doc! { "$and": docs }
        }
        FilterTree::Not(inner) => to_bson(inner),
    }
}

/// Convert a WIT scalar to a BSON value, matching how `bson::to_document` stores JSON data.
fn to_bson_value(v: &ScalarValue) -> Bson {
    match v {
        ScalarValue::Null => Bson::Null,
        ScalarValue::Boolean(b) => Bson::Boolean(*b),
        ScalarValue::Int32(i) => Bson::Int64(i64::from(*i)),
        ScalarValue::Int64(i) => Bson::Int64(*i),
        ScalarValue::Float64(f) => Bson::Double(*f),
        ScalarValue::Str(s) => Bson::String(s.clone()),
        ScalarValue::Binary(b) => Bson::Binary(bson::Binary {
            subtype: bson::spec::BinarySubtype::Generic,
            bytes: b.clone(),
        }),
        ScalarValue::Timestamp(t) => Bson::String(t.clone()),
    }
}

fn to_regex(pattern: &str) -> bson::Regex {
    bson::Regex {
        pattern: pattern.to_string(),
        options: String::new(),
    }
}

fn regex_escape(s: &str) -> String {
    regex::escape(s)
}

/// Returns `true` if the filter contains `StartsWith` or `EndsWith` nodes
/// that require client-side post-filtering (`PoloDB` ignores regex anchors).
pub fn needs_post_filter(tree: &FilterTree) -> bool {
    match tree {
        FilterTree::StartsWith { .. } | FilterTree::EndsWith { .. } => true,
        FilterTree::And(children) | FilterTree::Or(children) => {
            children.iter().any(needs_post_filter)
        }
        FilterTree::Not(inner) => needs_post_filter(inner),
        _ => false,
    }
}

/// Evaluate `StartsWith`/`EndsWith` conditions against a BSON document.
///
/// Other filter types return `true` (already evaluated by `PoloDB`).
pub fn post_filter_matches(tree: &FilterTree, doc: &bson::Document) -> bool {
    match tree {
        FilterTree::StartsWith { field, pattern } => {
            doc.get_str(field).is_ok_and(|s| s.starts_with(pattern.as_str()))
        }
        FilterTree::EndsWith { field, pattern } => {
            doc.get_str(field).is_ok_and(|s| s.ends_with(pattern.as_str()))
        }
        FilterTree::And(children) => children.iter().all(|c| post_filter_matches(c, doc)),
        FilterTree::Or(children) => children.iter().any(|c| post_filter_matches(c, doc)),
        FilterTree::Not(inner) => !post_filter_matches(inner, doc),
        _ => true,
    }
}
