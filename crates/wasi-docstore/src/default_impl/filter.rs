//! How the default in-memory backend applies a query's
//! [`Filter`](crate::Filter) and order to stored JSON.
//!
//! - A field path is dotted, so `"a.b"` descends into nested objects, and a
//!   missing field reads as JSON `null`.
//! - A test compares a stored value only with a value of the same type, so
//!   numbers with numbers and text with text. A stored value of any other
//!   type fails the test.
//! - Sorting orders values of different JSON types by a fixed type rank.

use std::cmp::Ordering;

use serde_json::Value;

use crate::{Condition, Filter, SortField, Test, Value as Operand};

// the field of the first `array-contains` in `filter`, a test this backend
// cannot serve
pub fn array_test(filter: &Filter) -> Option<&str> {
    let condition = filter.iter().flatten().find(|c| matches!(c.test, Test::ArrayContains(_)));
    condition.map(|condition| condition.field.as_str())
}

pub fn matches(filter: &Filter, doc: &Value) -> bool {
    filter.iter().all(|group| group.iter().any(|condition| holds(condition, doc)))
}

fn holds(condition: &Condition, doc: &Value) -> bool {
    passes(&condition.test, lookup(doc, &condition.field)) != condition.negated
}

fn passes(test: &Test, stored: &Value) -> bool {
    let equal = |value: &Operand| scalar_cmp(stored, value) == Some(Ordering::Equal);
    match test {
        Test::Eq(value) => equal(value),
        Test::Lt(value) => scalar_cmp(stored, value) == Some(Ordering::Less),
        Test::Lte(value) => scalar_cmp(stored, value).is_some_and(Ordering::is_le),
        Test::Gt(value) => scalar_cmp(stored, value) == Some(Ordering::Greater),
        Test::Gte(value) => scalar_cmp(stored, value).is_some_and(Ordering::is_ge),
        Test::In(values) => values.iter().any(equal),
        Test::StartsWith(pattern) => {
            stored.as_str().is_some_and(|s| s.starts_with(pattern.as_str()))
        }
        Test::EndsWith(pattern) => stored.as_str().is_some_and(|s| s.ends_with(pattern.as_str())),
        Test::Contains(pattern) => stored.as_str().is_some_and(|s| s.contains(pattern.as_str())),
        Test::Null => stored.is_null(),
        Test::ArrayContains(_) => {
            unreachable!("`query` refuses `array-contains` before it filters")
        }
    }
}

// the values `doc` orders by, one per field of `order_by`
pub fn sort_key(doc: &Value, order_by: &[SortField]) -> Vec<Value> {
    order_by.iter().map(|sort| lookup(doc, &sort.field).clone()).collect()
}

// Values of different JSON types order by a fixed type rank
// (null < bool < number < string < array < object), so the sort is total and
// deterministic. Callers break remaining ties on the document id.
pub fn compare_keys(a: &[Value], b: &[Value], order_by: &[SortField]) -> Ordering {
    for ((a, b), sort) in a.iter().zip(b).zip(order_by) {
        let ord = json_cmp(a, b);
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

// Compare a stored JSON value with a test's value; `None` = incomparable.
fn scalar_cmp(stored: &Value, target: &Operand) -> Option<Ordering> {
    match (stored, target) {
        (Value::Bool(a), Operand::Boolean(b)) => Some(a.cmp(b)),
        (Value::Number(n), Operand::Integer(i)) => {
            #[expect(clippy::cast_precision_loss, reason = "dev backend accepts f64 range")]
            let target = *i as f64;
            n.as_f64().and_then(|stored| stored.partial_cmp(&target))
        }
        (Value::Number(n), Operand::Number(f)) => {
            n.as_f64().and_then(|stored| stored.partial_cmp(f))
        }
        (Value::String(s), Operand::Text(t)) => Some(s.as_str().cmp(t.as_str())),
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
