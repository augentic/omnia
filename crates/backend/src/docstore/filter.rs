//! Pure JSON evaluation of [`Filter`] and shared in-memory
//! collection storage for `DocStoreDefault` and `omnia_test::guest::MemoryDocs`.

use std::cmp::Ordering;
use std::collections::btree_map::Entry;
use std::collections::{BTreeMap, HashMap};
use std::fmt::Debug;
use std::sync::{Arc, RwLock};

use anyhow::{Context as _, Result};
use serde_json::Value;

use super::{ComparisonOp, Document, Filter, QueryOptions, QueryResult, ScalarValue, SortField};

const MAX_PAGE_SIZE: u64 = 1000;

type Collections = HashMap<String, BTreeMap<String, Value>>;

/// Shared in-memory document collection store with [`Filter`] evaluation,
/// sorting, and offset/continuation pagination.
#[derive(Clone, Default)]
pub struct MemoryStore {
    collections: Arc<RwLock<Collections>>,
}

impl Debug for MemoryStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MemoryStore").finish_non_exhaustive()
    }
}

impl MemoryStore {
    fn read(&self) -> std::sync::RwLockReadGuard<'_, Collections> {
        self.collections.read().expect("docstore lock poisoned")
    }

    fn write(&self) -> std::sync::RwLockWriteGuard<'_, Collections> {
        self.collections.write().expect("docstore lock poisoned")
    }

    /// Point read by primary id.
    ///
    /// # Errors
    ///
    /// Returns an error if the stored JSON body cannot be serialized.
    pub fn get(&self, collection: &str, id: &str) -> Result<Option<Document>> {
        self.read()
            .get(collection)
            .and_then(|col| col.get(id))
            .map(|body| to_document(id, body))
            .transpose()
    }

    /// Insert a new document; fails if `doc.id` already exists in `collection`.
    ///
    /// # Errors
    ///
    /// Returns an error if `doc.data` is not valid JSON or `doc.id` already exists.
    pub fn insert(&self, collection: &str, doc: &Document) -> Result<()> {
        let body = parse_body(doc)?;
        let inserted =
            match self.write().entry(collection.to_owned()).or_default().entry(doc.id.clone()) {
                Entry::Vacant(slot) => {
                    slot.insert(body);
                    true
                }
                Entry::Occupied(_) => false,
            };
        anyhow::ensure!(inserted, "document id already exists");
        Ok(())
    }

    /// Upsert a document by id.
    ///
    /// # Errors
    ///
    /// Returns an error if `doc.data` is not valid JSON.
    pub fn put(&self, collection: &str, doc: &Document) -> Result<()> {
        let body = parse_body(doc)?;
        self.write().entry(collection.to_owned()).or_default().insert(doc.id.clone(), body);
        Ok(())
    }

    /// Delete a document by id; returns `true` if a document was removed.
    #[must_use]
    pub fn delete(&self, collection: &str, id: &str) -> bool {
        self.write().get_mut(collection).is_some_and(|col| col.remove(id).is_some())
    }

    /// Query documents in `collection` with filtering, sorting, and pagination.
    ///
    /// # Errors
    ///
    /// Returns an error if `options.limit` is zero or `options.continuation` is malformed.
    pub fn query(&self, collection: &str, options: &QueryOptions) -> Result<QueryResult> {
        let limit = options.limit.map_or(MAX_PAGE_SIZE, u64::from).min(MAX_PAGE_SIZE);
        anyhow::ensure!(limit > 0, "query limit must be at least 1");
        let page_size = usize::try_from(limit).unwrap_or(usize::MAX);

        let skip = parse_continuation(options.continuation.as_deref())?
            .saturating_add(u64::from(options.offset.unwrap_or(0)));
        let skip = usize::try_from(skip).unwrap_or(usize::MAX);

        let mut rows: Vec<(String, Value)> = {
            let guard = self.read();
            guard.get(collection).map_or_else(Vec::new, |col| {
                col.iter()
                    .filter(|(_, body)| options.filter.as_ref().is_none_or(|f| matches(f, body)))
                    .map(|(id, body)| (id.clone(), body.clone()))
                    .collect()
            })
        };

        if !options.order_by.is_empty() {
            rows.sort_by(|a, b| {
                compare_documents(&a.1, &b.1, &options.order_by).then_with(|| a.0.cmp(&b.0))
            });
        }

        let (page, continuation) = paginate(rows, skip, page_size);
        let documents =
            page.iter().map(|(id, body)| to_document(id, body)).collect::<Result<Vec<_>>>()?;

        Ok(QueryResult {
            documents,
            continuation,
        })
    }
}

fn paginate(
    rows: Vec<(String, Value)>, skip: usize, page_size: usize,
) -> (Vec<(String, Value)>, Option<String>) {
    let total = rows.len();
    let page: Vec<_> = rows.into_iter().skip(skip).take(page_size).collect();
    let next = skip.saturating_add(page.len());
    let continuation = (next < total).then(|| next.to_string());
    (page, continuation)
}

fn parse_continuation(continuation: Option<&str>) -> Result<u64> {
    continuation.map_or_else(
        || Ok(0),
        |s| s.parse().with_context(|| format!("invalid continuation token: {s:?}")),
    )
}

fn parse_body(doc: &Document) -> Result<Value> {
    serde_json::from_slice(&doc.data).context("invalid JSON in document body")
}

fn to_document(id: &str, body: &Value) -> Result<Document> {
    Ok(Document {
        id: id.to_owned(),
        data: serde_json::to_vec(body).context("serialize JSON body")?,
    })
}

/// Evaluate `tree` against a JSON document body.
#[must_use]
pub fn matches(tree: &Filter, doc: &Value) -> bool {
    match tree {
        Filter::Compare { field, op, value } => {
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
        Filter::InList { field, values } => {
            let stored = lookup(doc, field);
            values.iter().any(|v| scalar_cmp(stored, v) == Some(Ordering::Equal))
        }
        Filter::NotInList { field, values } => {
            let stored = lookup(doc, field);
            values.iter().all(|v| scalar_cmp(stored, v) != Some(Ordering::Equal))
        }
        Filter::IsNull(field) => lookup(doc, field).is_null(),
        Filter::IsNotNull(field) => !lookup(doc, field).is_null(),
        Filter::Contains { field, pattern } => {
            lookup(doc, field).as_str().is_some_and(|s| s.contains(pattern))
        }
        Filter::StartsWith { field, pattern } => {
            lookup(doc, field).as_str().is_some_and(|s| s.starts_with(pattern))
        }
        Filter::EndsWith { field, pattern } => {
            lookup(doc, field).as_str().is_some_and(|s| s.ends_with(pattern))
        }
        Filter::And(children) => children.iter().all(|c| matches(c, doc)),
        Filter::Or(children) => children.iter().any(|c| matches(c, doc)),
        Filter::Not(inner) => !matches(inner, doc),
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
        (Value::Bool(a), ScalarValue::Bool(b)) => Some(a.cmp(b)),
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

    fn compare(field: &str, op: ComparisonOp, value: ScalarValue) -> Filter {
        Filter::Compare {
            field: field.to_string(),
            op,
            value,
        }
    }

    #[test]
    fn and_eq_with_not_null() {
        let docs = [
            json!({"wb": 1, "zone_id": "z1"}),
            json!({"wb": 1, "zone_id": "z2"}),
            json!({"wb": 0, "zone_id": "z3"}),
            json!({"wb": 1, "zone_id": null}),
        ];
        let filter = Filter::And(vec![
            compare("wb", ComparisonOp::Eq, ScalarValue::Int32(1)),
            Filter::IsNotNull("zone_id".to_string()),
        ]);
        let matched = docs.iter().filter(|d| matches(&filter, d)).count();
        assert_eq!(matched, 2, "wb=1 AND zone_id IS NOT NULL");
    }

    #[test]
    fn ne_matches_missing_field() {
        let filter = compare("zone_id", ComparisonOp::Ne, ScalarValue::Str("z1".into()));
        assert!(matches(&filter, &json!({})), "missing field is not equal to a string");
        assert!(matches(&filter, &json!({"zone_id": null})), "null is not equal to a string");
        assert!(!matches(&filter, &json!({"zone_id": "z1"})));
    }

    #[test]
    fn ordering_needs_comparable() {
        let filter = compare("lat", ComparisonOp::Gte, ScalarValue::Float64(1.0));
        assert!(matches(&filter, &json!({"lat": 1.5})));
        assert!(!matches(&filter, &json!({"lat": "1.5"})), "string does not order against number");
        assert!(!matches(&filter, &json!({})), "missing field does not order");
    }

    #[test]
    fn starts_with_and_ends_with() {
        let doc = json!({"name": "Northern Line"});
        let starts = Filter::StartsWith {
            field: "name".to_string(),
            pattern: "Northern".to_string(),
        };
        let ends = Filter::EndsWith {
            field: "name".to_string(),
            pattern: "Line".to_string(),
        };
        assert!(matches(&starts, &doc));
        assert!(matches(&ends, &doc));
        assert!(!matches(
            &Filter::StartsWith {
                field: "name".to_string(),
                pattern: "Line".to_string(),
            },
            &doc
        ));
    }

    #[test]
    fn not_in_list() {
        let filter = Filter::NotInList {
            field: "route_type".to_string(),
            values: vec![ScalarValue::Int32(1), ScalarValue::Int32(2)],
        };
        assert!(matches(&filter, &json!({"route_type": 3})));
        assert!(!matches(&filter, &json!({"route_type": 2})));
        assert!(matches(&filter, &json!({})), "missing field is in no list");
    }

    #[test]
    fn contains_substring() {
        let filter = Filter::Contains {
            field: "name".to_string(),
            pattern: "Station".to_string(),
        };
        assert!(matches(&filter, &json!({"name": "Newmarket Station"})));
        assert!(!matches(&filter, &json!({"name": "Britomart"})));
        assert!(!matches(&filter, &json!({"name": 42})), "non-string never contains");
        assert!(!matches(&filter, &json!({})), "missing field never contains");
    }

    #[test]
    fn in_list_membership() {
        let filter = Filter::InList {
            field: "route_type".to_string(),
            values: vec![ScalarValue::Int32(2), ScalarValue::Int32(3)],
        };
        assert!(matches(&filter, &json!({"route_type": 2})));
        assert!(!matches(&filter, &json!({"route_type": 4})));
        assert!(!matches(&filter, &json!({})), "missing field is in no list");
    }

    #[test]
    fn is_null() {
        let filter = Filter::IsNull("parent".to_string());
        assert!(matches(&filter, &json!({"parent": null})));
        assert!(matches(&filter, &json!({})), "absent field reads as null");
        assert!(!matches(&filter, &json!({"parent": "stop-1"})));
    }

    #[test]
    fn or_across_fields() {
        let filter = Filter::Or(vec![
            Filter::Contains {
                field: "short_name".to_string(),
                pattern: "NEX".to_string(),
            },
            Filter::Contains {
                field: "long_name".to_string(),
                pattern: "Northern".to_string(),
            },
        ]);
        assert!(matches(&filter, &json!({"short_name": "NEX", "long_name": "x"})));
        assert!(matches(&filter, &json!({"short_name": "x", "long_name": "Northern Express"})));
        assert!(!matches(&filter, &json!({"short_name": "x", "long_name": "y"})));
    }

    #[test]
    fn not_of_and_conjunction() {
        // de morgan: everything but a document satisfying both conjuncts
        let filter = Filter::Not(Box::new(Filter::And(vec![
            compare("agency", ComparisonOp::Eq, ScalarValue::Str("AT".into())),
            compare("route_type", ComparisonOp::Eq, ScalarValue::Int32(3)),
        ])));
        assert!(!matches(&filter, &json!({"agency": "AT", "route_type": 3})));
        assert!(matches(&filter, &json!({"agency": "AT", "route_type": 2})));
        assert!(matches(&filter, &json!({"agency": "Fullers", "route_type": 3})));
        assert!(matches(&filter, &json!({})));
    }

    #[test]
    fn nested_path_lookup() {
        let doc = json!({"stop": {"zone": {"id": "z9"}}});
        let filter = compare("stop.zone.id", ComparisonOp::Eq, ScalarValue::Str("z9".into()));
        assert!(matches(&filter, &doc));
    }

    #[test]
    fn multi_field_desc_sort() {
        let order = vec![
            SortField {
                field: "a".to_string(),
                descending: false,
            },
            SortField {
                field: "b".to_string(),
                descending: true,
            },
        ];
        let low = json!({"a": 1, "b": 1});
        let high_b = json!({"a": 1, "b": 2});
        let high_a = json!({"a": 2, "b": 0});

        assert_eq!(compare_documents(&high_b, &low, &order), Ordering::Less, "b descends");
        assert_eq!(compare_documents(&low, &high_a, &order), Ordering::Less, "a ascends first");
    }

    #[test]
    fn mixed_types_sort_by_rank() {
        let order = vec![SortField {
            field: "v".to_string(),
            descending: false,
        }];
        let null = json!({});
        let number = json!({"v": 5});
        let string = json!({"v": "5"});

        assert_eq!(compare_documents(&null, &number, &order), Ordering::Less);
        assert_eq!(compare_documents(&number, &string, &order), Ordering::Less);
    }

    fn rows(ids: &[&str]) -> Vec<(String, Value)> {
        ids.iter().map(|id| ((*id).to_string(), json!({ "id": id }))).collect()
    }

    #[test]
    fn paginate_walks_all_pages() {
        let (page, cont) = paginate(rows(&["a", "b", "c", "d", "e"]), 0, 2);
        assert_eq!(page.len(), 2);
        assert_eq!(cont.as_deref(), Some("2"));

        let (page, cont) = paginate(rows(&["a", "b", "c", "d", "e"]), 2, 2);
        assert_eq!(page.len(), 2);
        assert_eq!(cont.as_deref(), Some("4"));

        let (page, cont) = paginate(rows(&["a", "b", "c", "d", "e"]), 4, 2);
        assert_eq!(page.len(), 1);
        assert_eq!(cont, None, "final partial page carries no continuation");
    }

    #[test]
    fn paginate_exact_boundary() {
        let (page, cont) = paginate(rows(&["a", "b"]), 0, 2);
        assert_eq!(page.len(), 2);
        assert_eq!(cont, None, "a full final page carries no continuation");
    }

    #[test]
    fn continuation_rejects_junk() {
        parse_continuation(Some("not-a-number")).expect_err("garbage token is rejected");
        assert_eq!(parse_continuation(None).expect("empty token"), 0);
    }
}
