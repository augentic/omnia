//! Pure JSON evaluation of [`FilterTree`](crate::host::resource::FilterTree)
//! for the default in-memory backend.
//!
//! Semantics follow the backend-portable filter contract:
//! - Field paths are dotted (`"a.b"` descends into nested objects); a missing
//!   field reads as JSON `null`, so `is-null` matches absent fields and
//!   `ne` matches documents lacking the field.
//! - Ordering comparisons (`gt`/`gte`/`lt`/`lte`) only match when the stored
//!   value and the target are of comparable types (numbers with numbers,
//!   strings with strings); `eq`/`ne` treat incomparable types as unequal.
//! - `binary` scalars never match JSON-derived values (JSON has no binary
//!   type), mirroring the previous default backend.

use std::cmp::Ordering;

use serde_json::Value;

use crate::host::generated::omnia::docstore::types::{ComparisonOp, ScalarValue, SortField};
use crate::host::resource::FilterTree;

/// Evaluate `tree` against a JSON document body.
#[must_use]
pub fn matches(tree: &FilterTree, doc: &Value) -> bool {
    match tree {
        FilterTree::Compare { field, op, value } => {
            let stored = lookup(doc, field);
            match op {
                ComparisonOp::Eq => scalar_cmp(stored, value) == Some(Ordering::Equal),
                ComparisonOp::Ne => scalar_cmp(stored, value) != Some(Ordering::Equal),
                ComparisonOp::Gt => scalar_cmp(stored, value) == Some(Ordering::Greater),
                ComparisonOp::Gte => {
                    matches!(scalar_cmp(stored, value), Some(Ordering::Greater | Ordering::Equal))
                }
                ComparisonOp::Lt => scalar_cmp(stored, value) == Some(Ordering::Less),
                ComparisonOp::Lte => {
                    matches!(scalar_cmp(stored, value), Some(Ordering::Less | Ordering::Equal))
                }
            }
        }
        FilterTree::InList { field, values } => {
            let stored = lookup(doc, field);
            values.iter().any(|v| scalar_cmp(stored, v) == Some(Ordering::Equal))
        }
        FilterTree::NotInList { field, values } => {
            let stored = lookup(doc, field);
            values.iter().all(|v| scalar_cmp(stored, v) != Some(Ordering::Equal))
        }
        FilterTree::IsNull(field) => lookup(doc, field).is_null(),
        FilterTree::IsNotNull(field) => !lookup(doc, field).is_null(),
        FilterTree::Contains { field, pattern } => {
            lookup(doc, field).as_str().is_some_and(|s| s.contains(pattern))
        }
        FilterTree::StartsWith { field, pattern } => {
            lookup(doc, field).as_str().is_some_and(|s| s.starts_with(pattern))
        }
        FilterTree::EndsWith { field, pattern } => {
            lookup(doc, field).as_str().is_some_and(|s| s.ends_with(pattern))
        }
        FilterTree::And(children) => children.iter().all(|c| matches(c, doc)),
        FilterTree::Or(children) => children.iter().any(|c| matches(c, doc)),
        FilterTree::Not(inner) => !matches(inner, doc),
    }
}

/// Compare two documents under `order_by`, field by field.
///
/// Values of different JSON types order by a fixed type rank
/// (null < bool < number < string < array < object) so the sort is total and
/// deterministic; callers break remaining ties on the document id.
#[must_use]
pub fn compare_documents(a: &Value, b: &Value, order_by: &[SortField]) -> Ordering {
    for sort in order_by {
        let ord = json_cmp(lookup(a, &sort.field), lookup(b, &sort.field));
        let ord = if sort.descending { ord.reverse() } else { ord };
        if ord != Ordering::Equal {
            return ord;
        }
    }
    Ordering::Equal
}

const NULL: Value = Value::Null;

// Resolve a dotted path inside a JSON document; absent segments read as null.
fn lookup<'a>(doc: &'a Value, path: &str) -> &'a Value {
    let mut current = doc;
    for segment in path.split('.') {
        match current.get(segment) {
            Some(value) => current = value,
            None => return &NULL,
        }
    }
    current
}

// Compare a stored JSON value with a target scalar; `None` = incomparable.
fn scalar_cmp(stored: &Value, target: &ScalarValue) -> Option<Ordering> {
    match (stored, target) {
        (Value::Null, ScalarValue::Null) => Some(Ordering::Equal),
        (Value::Bool(a), ScalarValue::Boolean(b)) => Some(a.cmp(b)),
        (Value::Number(n), _) => {
            let target = match target {
                ScalarValue::Int32(i) => f64::from(*i),
                #[expect(clippy::cast_precision_loss, reason = "dev backend accepts f64 range")]
                ScalarValue::Int64(i) => *i as f64,
                ScalarValue::Float64(f) => *f,
                _ => return None,
            };
            n.as_f64().and_then(|stored| stored.partial_cmp(&target))
        }
        (Value::String(s), ScalarValue::Str(t) | ScalarValue::Timestamp(t)) => {
            Some(s.as_str().cmp(t.as_str()))
        }
        _ => None,
    }
}

// Total order over JSON values: type rank first, then value.
fn json_cmp(a: &Value, b: &Value) -> Ordering {
    const fn rank(v: &Value) -> u8 {
        match v {
            Value::Null => 0,
            Value::Bool(_) => 1,
            Value::Number(_) => 2,
            Value::String(_) => 3,
            Value::Array(_) => 4,
            Value::Object(_) => 5,
        }
    }

    match (a, b) {
        (Value::Bool(x), Value::Bool(y)) => x.cmp(y),
        (Value::Number(x), Value::Number(y)) => x
            .as_f64()
            .zip(y.as_f64())
            .and_then(|(x, y)| x.partial_cmp(&y))
            .unwrap_or(Ordering::Equal),
        (Value::String(x), Value::String(y)) => x.cmp(y),
        _ => rank(a).cmp(&rank(b)),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn absent_or_incomparable() {
        let gte = FilterTree::Compare {
            field: "v".to_string(),
            op: ComparisonOp::Gte,
            value: ScalarValue::Float64(1.0),
        };
        let in_list = FilterTree::InList {
            field: "v".to_string(),
            values: vec![ScalarValue::Int32(2)],
        };
        let not_in_list = FilterTree::NotInList {
            field: "v".to_string(),
            values: vec![ScalarValue::Int32(2)],
        };
        let contains = FilterTree::Contains {
            field: "v".to_string(),
            pattern: "5".to_string(),
        };

        assert!(!matches(&gte, &json!({"v": "1.5"})), "string does not order against number");
        assert!(!matches(&gte, &json!({})), "missing field does not order");
        assert!(!matches(&in_list, &json!({})), "missing field is in no list");
        assert!(matches(&not_in_list, &json!({})), "missing field is in no list");
        assert!(!matches(&contains, &json!({"v": 15})), "non-string never contains");
        assert!(!matches(&contains, &json!({})), "missing field never contains");
    }

    #[test]
    fn null_sorts_first() {
        let order = [SortField {
            field: "v".to_string(),
            descending: false,
        }];
        assert_eq!(compare_documents(&json!({}), &json!({"v": 5}), &order), Ordering::Less);
    }
}
