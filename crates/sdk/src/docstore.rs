//! Document store value types, filter builders, and the reference query semantics.
//!
//! [`Filter::matches`] and [`SortField::compare`] define how a query selects
//! and orders JSON documents. A provider that evaluates queries itself calls
//! them rather than writing its own evaluator.

use std::cmp::Ordering;
use std::ops::Not;

use bytes::Bytes;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::capability::{Error, ErrorKind, Result};

/// Scalar values for filter comparisons.
#[derive(Debug, Clone, PartialEq)]
pub enum ScalarValue {
    /// Null literal.
    Null,
    /// Boolean.
    Bool(bool),
    /// 32-bit integer.
    Int32(i32),
    /// 64-bit integer.
    Int64(i64),
    /// Floating point.
    Float64(f64),
    /// UTF-8 string.
    Str(String),
    /// Opaque bytes.
    Binary(Vec<u8>),
    /// ISO-8601 timestamp string for comparisons.
    Timestamp(String),
}

impl From<&str> for ScalarValue {
    fn from(s: &str) -> Self {
        Self::Str(s.to_string())
    }
}

impl From<String> for ScalarValue {
    fn from(s: String) -> Self {
        Self::Str(s)
    }
}

impl From<i32> for ScalarValue {
    fn from(v: i32) -> Self {
        Self::Int32(v)
    }
}

impl From<i64> for ScalarValue {
    fn from(v: i64) -> Self {
        Self::Int64(v)
    }
}

impl From<f64> for ScalarValue {
    fn from(v: f64) -> Self {
        Self::Float64(v)
    }
}

impl From<bool> for ScalarValue {
    fn from(v: bool) -> Self {
        Self::Bool(v)
    }
}

impl From<Timestamp> for ScalarValue {
    fn from(t: Timestamp) -> Self {
        Self::Timestamp(t.0)
    }
}

/// Comparison operators for filters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComparisonOp {
    /// Equal.
    Eq,
    /// Not equal.
    Ne,
    /// Greater than.
    Gt,
    /// Greater than or equal.
    Gte,
    /// Less than.
    Lt,
    /// Less than or equal.
    Lte,
}

/// A filter expression tree.
#[derive(Debug, Clone)]
pub enum Filter {
    /// Compare `field` to `value` using `op`.
    Compare {
        /// Field path.
        field: String,
        /// Comparison operator.
        op: ComparisonOp,
        /// Right-hand value.
        value: ScalarValue,
    },
    /// Field value is one of the given values.
    InList {
        /// Field path.
        field: String,
        /// Allowed values.
        values: Vec<ScalarValue>,
    },
    /// Field value is not in the given set.
    NotInList {
        /// Field path.
        field: String,
        /// Excluded values.
        values: Vec<ScalarValue>,
    },
    /// Field is null or missing.
    IsNull(String),
    /// Field exists and is not null.
    IsNotNull(String),
    /// String contains pattern (backend-defined semantics).
    Contains {
        /// Field path.
        field: String,
        /// Substring pattern.
        pattern: String,
    },
    /// String starts with pattern.
    StartsWith {
        /// Field path.
        field: String,
        /// Prefix pattern.
        pattern: String,
    },
    /// String ends with pattern.
    EndsWith {
        /// Field path.
        field: String,
        /// Suffix pattern.
        pattern: String,
    },
    /// Logical AND of child filters.
    And(Vec<Self>),
    /// Logical OR of child filters.
    Or(Vec<Self>),
    /// Logical NOT.
    Not(Box<Self>),
}

impl Filter {
    /// The deepest a filter tree may nest, as [`Filter::depth`] counts it.
    pub const MAX_DEPTH: usize = 5;
    /// The most values an [`InList`](Self::InList) or [`NotInList`](Self::NotInList) may hold.
    pub const MAX_LIST_LEN: usize = 100;

    /// Compare `field` to `val` with `op`.
    #[must_use]
    pub fn cmp(field: &str, op: ComparisonOp, val: impl Into<ScalarValue>) -> Self {
        Self::Compare {
            field: field.to_string(),
            op,
            value: val.into(),
        }
    }

    /// Equality comparison.
    #[must_use]
    pub fn eq(field: &str, val: impl Into<ScalarValue>) -> Self {
        Self::cmp(field, ComparisonOp::Eq, val)
    }

    /// Inequality comparison.
    #[must_use]
    pub fn ne(field: &str, val: impl Into<ScalarValue>) -> Self {
        Self::cmp(field, ComparisonOp::Ne, val)
    }

    /// Greater than.
    #[must_use]
    pub fn gt(field: &str, val: impl Into<ScalarValue>) -> Self {
        Self::cmp(field, ComparisonOp::Gt, val)
    }

    /// Greater than or equal.
    #[must_use]
    pub fn gte(field: &str, val: impl Into<ScalarValue>) -> Self {
        Self::cmp(field, ComparisonOp::Gte, val)
    }

    /// Less than.
    #[must_use]
    pub fn lt(field: &str, val: impl Into<ScalarValue>) -> Self {
        Self::cmp(field, ComparisonOp::Lt, val)
    }

    /// Less than or equal.
    #[must_use]
    pub fn lte(field: &str, val: impl Into<ScalarValue>) -> Self {
        Self::cmp(field, ComparisonOp::Lte, val)
    }

    /// Field value is in the given set.
    #[must_use]
    pub fn in_list(field: &str, vals: impl IntoIterator<Item = impl Into<ScalarValue>>) -> Self {
        Self::InList {
            field: field.to_string(),
            values: vals.into_iter().map(Into::into).collect(),
        }
    }

    /// Field value is not in the given set.
    #[must_use]
    pub fn not_in_list(
        field: &str, vals: impl IntoIterator<Item = impl Into<ScalarValue>>,
    ) -> Self {
        Self::NotInList {
            field: field.to_string(),
            values: vals.into_iter().map(Into::into).collect(),
        }
    }

    /// Field is null or missing.
    #[must_use]
    pub fn is_null(field: &str) -> Self {
        Self::IsNull(field.to_string())
    }

    /// Field exists and is not null.
    #[must_use]
    pub fn is_not_null(field: &str) -> Self {
        Self::IsNotNull(field.to_string())
    }

    /// String contains pattern.
    #[must_use]
    pub fn contains(field: &str, pattern: &str) -> Self {
        Self::Contains {
            field: field.to_string(),
            pattern: pattern.to_string(),
        }
    }

    /// String starts with pattern.
    #[must_use]
    pub fn starts_with(field: &str, pattern: &str) -> Self {
        Self::StartsWith {
            field: field.to_string(),
            pattern: pattern.to_string(),
        }
    }

    /// String ends with pattern.
    #[must_use]
    pub fn ends_with(field: &str, pattern: &str) -> Self {
        Self::EndsWith {
            field: field.to_string(),
            pattern: pattern.to_string(),
        }
    }

    /// Logical AND.
    #[must_use]
    pub fn and(filters: impl IntoIterator<Item = Self>) -> Self {
        Self::And(filters.into_iter().collect())
    }

    /// Logical OR.
    #[must_use]
    pub fn or(filters: impl IntoIterator<Item = Self>) -> Self {
        Self::Or(filters.into_iter().collect())
    }

    /// Logical NOT.
    #[must_use]
    pub fn negate(inner: Self) -> Self {
        !inner
    }

    /// Restrict `field` to a calendar date (UTC day) using range on timestamp strings.
    ///
    /// # Errors
    ///
    /// Returns an error if `iso_date` is not a zero-padded `YYYY-MM-DD` date,
    /// or is `9999-12-31`, which has no next day in that form.
    pub fn on_date(field: &str, iso_date: &str) -> anyhow::Result<Self> {
        let next = next_iso_date(iso_date)?;
        let start = format!("{iso_date}T00:00:00Z");
        let end = format!("{next}T00:00:00Z");
        Ok(Self::And(vec![Self::gte(field, Timestamp(start)), Self::lt(field, Timestamp(end))]))
    }

    /// Maximum nesting depth of the filter tree.
    #[must_use]
    pub fn depth(&self) -> usize {
        match self {
            Self::And(children) | Self::Or(children) => {
                1 + children.iter().map(Self::depth).max().unwrap_or(0)
            }
            Self::Not(inner) => 1 + inner.depth(),
            _ => 1,
        }
    }

    /// Check that this filter keeps to the limits of every document store query.
    ///
    /// A provider checks a query's filter before it serves the query, so a
    /// filter past a limit fails the same way on every provider.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Invalid`] when the tree nests deeper than
    /// [`MAX_DEPTH`](Self::MAX_DEPTH), a list is empty or holds more than
    /// [`MAX_LIST_LEN`](Self::MAX_LIST_LEN) values, or an [`And`](Self::And)
    /// or [`Or`](Self::Or) has no children.
    pub fn check(&self) -> Result<()> {
        let depth = self.depth();
        if depth > Self::MAX_DEPTH {
            let message =
                format!("the filter nests {depth} deep, past the limit of {}", Self::MAX_DEPTH);
            return Err(Error::new(ErrorKind::Invalid, message));
        }
        self.check_node()
    }

    fn check_node(&self) -> Result<()> {
        match self {
            Self::InList { field, values } | Self::NotInList { field, values } => {
                if (1..=Self::MAX_LIST_LEN).contains(&values.len()) {
                    return Ok(());
                }
                let message = format!(
                    "the list filter on `{field}` holds {} values, not 1 to {}",
                    values.len(),
                    Self::MAX_LIST_LEN
                );
                Err(Error::new(ErrorKind::Invalid, message))
            }
            Self::And(children) | Self::Or(children) => {
                if children.is_empty() {
                    return Err(Error::new(ErrorKind::Invalid, "an `and` or `or` has no children"));
                }
                children.iter().try_for_each(Self::check_node)
            }
            Self::Not(inner) => inner.check_node(),
            _ => Ok(()),
        }
    }

    /// Whether the JSON document body `doc` satisfies this filter.
    ///
    /// These are the semantics every document store provider follows:
    ///
    /// - A field path is dotted: `"a.b"` descends into nested objects.
    /// - A missing field reads as `null`, so [`Filter::IsNull`] matches an
    ///   absent field and [`ComparisonOp::Ne`] matches a document lacking it.
    /// - An ordering comparison matches only comparable types: a number with
    ///   a number, a string with a [`ScalarValue::Str`] or
    ///   [`ScalarValue::Timestamp`]. Equality treats incomparable types as
    ///   unequal.
    /// - A [`ScalarValue::Binary`] never matches, since JSON has no binary
    ///   type.
    /// - [`Filter::Contains`], [`Filter::StartsWith`], and
    ///   [`Filter::EndsWith`] match only string fields.
    #[must_use]
    pub fn matches(&self, doc: &Value) -> bool {
        match self {
            Self::Compare { field, op, value } => {
                let ordering = scalar_cmp(lookup(doc, field), value);
                match op {
                    ComparisonOp::Eq => ordering == Some(Ordering::Equal),
                    ComparisonOp::Ne => ordering != Some(Ordering::Equal),
                    ComparisonOp::Gt => ordering == Some(Ordering::Greater),
                    ComparisonOp::Gte => {
                        matches!(ordering, Some(Ordering::Greater | Ordering::Equal))
                    }
                    ComparisonOp::Lt => ordering == Some(Ordering::Less),
                    ComparisonOp::Lte => matches!(ordering, Some(Ordering::Less | Ordering::Equal)),
                }
            }
            Self::InList { field, values } => {
                let stored = lookup(doc, field);
                values.iter().any(|value| scalar_cmp(stored, value) == Some(Ordering::Equal))
            }
            Self::NotInList { field, values } => {
                let stored = lookup(doc, field);
                values.iter().all(|value| scalar_cmp(stored, value) != Some(Ordering::Equal))
            }
            Self::IsNull(field) => lookup(doc, field).is_null(),
            Self::IsNotNull(field) => !lookup(doc, field).is_null(),
            Self::Contains { field, pattern } => {
                lookup(doc, field).as_str().is_some_and(|s| s.contains(pattern))
            }
            Self::StartsWith { field, pattern } => {
                lookup(doc, field).as_str().is_some_and(|s| s.starts_with(pattern))
            }
            Self::EndsWith { field, pattern } => {
                lookup(doc, field).as_str().is_some_and(|s| s.ends_with(pattern))
            }
            Self::And(children) => children.iter().all(|child| child.matches(doc)),
            Self::Or(children) => children.iter().any(|child| child.matches(doc)),
            Self::Not(inner) => !inner.matches(doc),
        }
    }
}

impl Not for Filter {
    type Output = Self;

    fn not(self) -> Self::Output {
        Self::Not(Box::new(self))
    }
}

fn next_iso_date(iso_date: &str) -> anyhow::Result<String> {
    let (year, month, day) = parse_iso_date(iso_date)
        .ok_or_else(|| anyhow::anyhow!("invalid ISO date: {iso_date:?}"))?;
    let (year, month, day) = if day < days_in_month(year, month) {
        (year, month, day + 1)
    } else if month < 12 {
        (year, month + 1, 1)
    } else if year < 9999 {
        (year + 1, 1, 1)
    } else {
        anyhow::bail!("cannot compute next day for date: {iso_date}");
    };
    Ok(format!("{year:04}-{month:02}-{day:02}"))
}

fn parse_iso_date(iso_date: &str) -> Option<(u32, u32, u32)> {
    let mut parts = iso_date.split('-');
    let year = digits(parts.next()?, 4)?;
    let month = digits(parts.next()?, 2)?;
    let day = digits(parts.next()?, 2)?;
    let valid = parts.next().is_none()
        && (1..=12).contains(&month)
        && (1..=days_in_month(year, month)).contains(&day);
    valid.then_some((year, month, day))
}

fn digits(part: &str, width: usize) -> Option<u32> {
    if part.len() == width && part.bytes().all(|byte| byte.is_ascii_digit()) {
        part.parse().ok()
    } else {
        None
    }
}

const fn days_in_month(year: u32, month: u32) -> u32 {
    match month {
        2 if year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400)) => {
            29
        }
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

const NULL: Value = Value::Null;

fn lookup<'a>(doc: &'a Value, path: &str) -> &'a Value {
    path.split('.').try_fold(doc, |current, segment| current.get(segment)).unwrap_or(&NULL)
}

fn scalar_cmp(stored: &Value, target: &ScalarValue) -> Option<Ordering> {
    match (stored, target) {
        (Value::Null, ScalarValue::Null) => Some(Ordering::Equal),
        (Value::Bool(a), ScalarValue::Bool(b)) => Some(a.cmp(b)),
        (Value::Number(n), _) => {
            let target = match target {
                ScalarValue::Int32(i) => f64::from(*i),
                #[expect(clippy::cast_precision_loss, reason = "JSON numbers compare as f64")]
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

/// Sort field for queries.
#[derive(Debug, Clone, Default)]
pub struct SortField {
    /// Field path.
    pub field: String,
    /// When `true`, sort descending.
    pub descending: bool,
}

impl SortField {
    /// Order two JSON document bodies by this field.
    ///
    /// The field path reads as in [`Filter::matches`]. Values of different
    /// JSON types order by type, `null` < bool < number < string < array <
    /// object, so the order is total. Two arrays, or two objects, are equal.
    /// A descending field reverses the order.
    ///
    /// A query sorts by its first field, then by each next field with
    /// [`Ordering::then_with`], and breaks the remaining ties on the document
    /// id so its pages are stable.
    #[must_use]
    pub fn compare(&self, a: &Value, b: &Value) -> Ordering {
        let ordering = json_cmp(lookup(a, &self.field), lookup(b, &self.field));
        if self.descending { ordering.reverse() } else { ordering }
    }
}

fn json_cmp(a: &Value, b: &Value) -> Ordering {
    const fn rank(value: &Value) -> u8 {
        match value {
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

/// A query over the documents in a collection.
#[derive(Debug, Clone, Default)]
pub struct Query {
    /// Optional filter tree.
    pub filter: Option<Filter>,

    /// Sort order (first key wins, then next, etc.).
    pub order_by: Vec<SortField>,

    /// Maximum documents to return.
    pub limit: Option<u32>,

    /// Skip this many documents after filter/sort (offset pagination).
    pub offset: Option<u32>,

    /// The [`next`](crate::capability::Page::next) cursor of the previous page,
    /// or `None` for the first page.
    pub after: Option<String>,
}

/// Stored document: identifier plus JSON body bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Document {
    /// Primary key string.
    pub id: String,

    /// JSON payload.
    pub data: Bytes,
}

impl Document {
    /// The document `id` names, with `data` as its JSON body.
    pub fn new(id: impl Into<String>, data: &[u8]) -> Self {
        Self {
            id: id.into(),
            data: Bytes::copy_from_slice(data),
        }
    }

    /// The document `id` names, with `value` serialized as its JSON body.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Invalid`] when `value` does not serialize.
    pub fn json<T: Serialize + ?Sized>(id: impl Into<String>, value: &T) -> Result<Self> {
        let id = id.into();
        let data = serde_json::to_vec(value).map_err(|err| {
            Error::new(ErrorKind::Invalid, format!("encoding document `{id}`")).with_source(err)
        })?;
        Ok(Self {
            id,
            data: Bytes::from(data),
        })
    }

    /// Deserialize the JSON body as `T`.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Invalid`] when the body is not the JSON of a `T`.
    pub fn decode<T: DeserializeOwned>(&self) -> Result<T> {
        serde_json::from_slice(&self.data).map_err(|err| {
            let message = format!("decoding document `{}`", self.id);
            Error::new(ErrorKind::Invalid, message).with_source(err)
        })
    }
}

/// The outcome of an [`insert`](crate::DocumentDb::insert) whose id is already stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Exists;

/// Newtype for timestamp strings so `Filter::gte("ts", Timestamp(...))` is explicit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Timestamp(pub String);
