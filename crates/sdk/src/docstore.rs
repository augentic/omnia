//! `omnia:docstore`, the collections of JSON documents a deployment declares.
//!
//! A guest opens a [`Collection`] with [`open`]. Beside the generated
//! methods, a collection has forms that take borrowed ids:
//!
//! - [`get_json`](Collection::get_json) reads a document's JSON as a typed
//!   value, and [`read`](Collection::read) reads the raw [`Document`].
//! - [`insert`](Collection::insert) stores a document unless its id is
//!   stored, and [`upsert`](Collection::upsert) stores one unconditionally.
//! - [`remove`](Collection::remove) deletes a document, and
//!   [`contains`](Collection::contains) tells whether one is stored.
//! - [`find`](Collection::find) runs a [`Query`] for one page, and
//!   [`find_all`](Collection::find_all) for every page.
//!
//! [`Query::builder`] builds a query, and [`Filter::all`] joins the filters
//! a request asks for into one. [`Filter::matches`] tests a JSON document
//! against a filter, and [`SortField::compare`] orders two by a field.
//!
//! Every backend serves a portable query:
//!
//! - [`Filter::Compare`] with a value other than `null`, [`Filter::InList`],
//!   and [`Filter::NotInList`], on top-level fields;
//! - combined with [`Filter::And`], [`Filter::Or`], and [`Filter::Not`];
//! - at most 15 comparisons, counting each value of a list;
//! - an empty `order_by`;
//! - a `limit`, with the page's `next` cursor.
//!
//! A zero `limit`, or a filter [`Filter::check`] refuses, fails with the
//! code `invalid`. A backend serves every other feature as defined, or fails
//! the query with the code `unsupported`, and never ignores one. The
//! [document store reference](https://github.com/augentic/omnia/blob/main/docs/reference/docstore.md#query-contract)
//! lists the portable names and the optional features.

pub(crate) mod groups;

use std::cmp::Ordering;
use std::ops::Not;

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

pub use crate::bindings::imports::omnia::docstore::{collection, store, types};
pub use crate::docstore::collection::{Collection, Page, SortField};
pub use crate::docstore::types::Document;
use crate::error::unconditional;
use crate::{Error, Result};

/// Open the collection `name`, a binding the deployment declares.
///
/// # Errors
///
/// The host's failure, under its code: `not-found` for a collection the
/// deployment does not declare, here or at the first call on it.
pub async fn open(name: &str) -> Result<Collection> {
    Ok(store::open(name.to_owned()).await?)
}

impl Collection {
    /// The document `id` with its JSON decoded as `T`, or `None` when it is
    /// absent.
    ///
    /// # Errors
    ///
    /// - [`ServerError`](crate::ErrorKind::ServerError) with the code `invalid` when the
    ///   JSON does not decode as `T`.
    /// - The host's failure, under its code.
    pub async fn get_json<T: DeserializeOwned>(&self, id: &str) -> Result<Option<T>> {
        self.get(id.to_owned()).await?.map(|document| document.decode()).transpose()
    }

    /// The document `id` as it is stored, or `None` when it is absent.
    ///
    /// # Errors
    ///
    /// The host's failure, under its code.
    pub async fn read(&self, id: &str) -> Result<Option<Document>> {
        Ok(self.get(id.to_owned()).await?)
    }

    /// Whether a document is stored under `id`.
    ///
    /// # Errors
    ///
    /// The host's failure, under its code.
    pub async fn contains(&self, id: &str) -> Result<bool> {
        Ok(self.exists(id.to_owned()).await?)
    }

    /// Store `value` as the JSON of the document `id`, unless `id` is stored.
    ///
    /// A stored id answers `Ok(Err(Exists))` and leaves the stored document
    /// untouched.
    ///
    /// # Errors
    ///
    /// - [`ServerError`](crate::ErrorKind::ServerError) with the code `invalid` when
    ///   `value` does not serialize.
    /// - The host's failure, under its code.
    pub async fn insert<T: Serialize + Sync + ?Sized>(
        &self, id: &str, value: &T,
    ) -> Result<Result<(), Exists>> {
        let data = encode(id, value)?;
        let written = self.put(id.to_owned(), data, Some(types::Precondition::Absent)).await?;
        Ok(written.map(|_revision| ()).map_err(|_conflict| Exists))
    }

    /// Store `value` as the JSON of the document `id`, replacing any there.
    ///
    /// # Errors
    ///
    /// - [`ServerError`](crate::ErrorKind::ServerError) with the code `invalid` when
    ///   `value` does not serialize.
    /// - The host's failure, under its code.
    pub async fn upsert<T: Serialize + Sync + ?Sized>(&self, id: &str, value: &T) -> Result<()> {
        unconditional(self.put(id.to_owned(), encode(id, value)?, None).await?)?;
        Ok(())
    }

    /// Delete the document `id`, and answer whether one was removed.
    ///
    /// Deleting an absent document succeeds with `false`.
    ///
    /// # Errors
    ///
    /// The host's failure, under its code.
    pub async fn remove(&self, id: &str) -> Result<bool> {
        unconditional(self.delete(id.to_owned(), None).await?)
    }

    /// One page of the documents `query` selects.
    ///
    /// # Errors
    ///
    /// - [`ServerError`](crate::ErrorKind::ServerError) with the code `invalid` for a
    ///   filter [`Filter::check`] refuses.
    /// - The host's failure, under its code: `unsupported` for a feature the
    ///   backend does not serve.
    pub async fn find(&self, query: &Query) -> Result<Page> {
        let filter = match &query.filter {
            Some(filter) => filter.groups()?,
            None => Vec::new(),
        };
        let filter = filter.into_iter().map(|group| group.into_iter().map(Into::into).collect());
        let order_by = query.order_by.clone();
        Ok(self.query(filter.collect(), order_by, query.after.clone(), query.limit).await?)
    }

    /// Every document `query` selects, from its `after` cursor on, in the
    /// order it asks for.
    ///
    /// The query's `limit` sizes each page. A document unchanged for the
    /// whole walk is in it once, and one written or deleted during it may or
    /// may not be.
    ///
    /// # Errors
    ///
    /// As [`find`](Self::find), on any page.
    pub async fn find_all(&self, query: &Query) -> Result<Vec<Document>> {
        let mut query = query.clone();
        let mut documents = Vec::new();
        loop {
            let page = self.find(&query).await?;
            documents.extend(page.documents);
            match page.next {
                Some(next) => query.after = Some(next),
                None => return Ok(documents),
            }
        }
    }
}

impl types::Document {
    /// Deserialize the document's JSON as `T`.
    ///
    /// # Errors
    ///
    /// [`ServerError`](crate::ErrorKind::ServerError) with the code `invalid` when the
    /// JSON does not decode as `T`.
    pub fn decode<T: DeserializeOwned>(&self) -> Result<T> {
        decode(&self.id, self.data.as_bytes())
    }
}

fn encode<T: Serialize + ?Sized>(id: &str, value: &T) -> Result<String> {
    serde_json::to_string(value)
        .map_err(|err| Error::server_error("invalid", format!("encoding document `{id}`: {err}")))
}

fn decode<T: DeserializeOwned>(id: &str, data: &[u8]) -> Result<T> {
    serde_json::from_slice(data)
        .map_err(|err| Error::server_error("invalid", format!("decoding document `{id}`: {err}")))
}

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
    /// ISO-8601 timestamp string, compared as text.
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
    /// Field is an array, and one of its elements equals `value`.
    ArrayContains {
        /// Field path.
        field: String,
        /// The element to find.
        value: ScalarValue,
    },
    /// Logical AND of child filters.
    And(Vec<Self>),
    /// Logical OR of child filters.
    Or(Vec<Self>),
    /// Logical NOT.
    Not(Box<Self>),
}

impl Filter {
    /// The most conditions a filter may hold once written as an `and` of `or`s.
    ///
    /// [`Filter::check`] writes it so by pushing each [`Not`](Self::Not) down
    /// to the fields and distributing each [`Or`](Self::Or) over the
    /// [`And`](Self::And)s inside it. A list then counts once, or twice when
    /// it holds [`Null`](ScalarValue::Null). Every other leaf counts once.
    pub const MAX_CONDITIONS: usize = 100;
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

    /// Field is an array holding an element equal to `val`.
    #[must_use]
    pub fn array_contains(field: &str, val: impl Into<ScalarValue>) -> Self {
        Self::ArrayContains {
            field: field.to_string(),
            value: val.into(),
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

    /// The filter every one of `filters` must pass: `None` when there are
    /// none, the filter itself when there is one, and an [`And`](Self::And)
    /// of them all otherwise.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use omnia_sdk::docstore::{Filter, Query};
    ///
    /// let route = Some("r1");
    /// let stop: Option<&str> = None;
    /// let filters =
    ///     [route.map(|id| Filter::eq("route_id", id)), stop.map(|id| Filter::eq("stop_id", id))];
    ///
    /// let filter = Filter::all(filters.into_iter().flatten());
    /// let query = Query::builder().maybe_filter(filter).limit(50).build();
    ///
    /// assert!(matches!(query.filter, Some(Filter::Compare { .. })));
    /// ```
    #[must_use]
    pub fn all(filters: impl IntoIterator<Item = Self>) -> Option<Self> {
        let mut filters: Vec<Self> = filters.into_iter().collect();
        match filters.len() {
            0 => None,
            1 => filters.pop(),
            _ => Some(Self::And(filters)),
        }
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
    /// A [`ServerError`](crate::ErrorKind::ServerError) with the code `invalid` if
    /// `iso_date` is not a zero-padded `YYYY-MM-DD` date, or is `9999-12-31`,
    /// which has no next day in that form.
    pub fn on_date(field: &str, iso_date: &str) -> Result<Self> {
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
    /// A [`ServerError`](crate::ErrorKind::ServerError) with the code `invalid` when:
    ///
    /// - the tree nests deeper than [`MAX_DEPTH`](Self::MAX_DEPTH);
    /// - a list is empty or holds more than [`MAX_LIST_LEN`](Self::MAX_LIST_LEN) values;
    /// - an [`And`](Self::And) or [`Or`](Self::Or) has no children;
    /// - the filter holds more than [`MAX_CONDITIONS`](Self::MAX_CONDITIONS) conditions;
    /// - a value is a [`Float64`](ScalarValue::Float64) that is not finite;
    /// - an ordering comparison is given [`Null`](ScalarValue::Null) or a
    ///   [`Bool`](ScalarValue::Bool);
    /// - an [`ArrayContains`](Self::ArrayContains) is given
    ///   [`Null`](ScalarValue::Null).
    pub fn check(&self) -> Result<()> {
        self.groups()?;
        Ok(())
    }

    /// Whether the JSON document body `doc` satisfies this filter.
    ///
    /// - A field path is dotted: `"a.b"` descends into nested objects.
    /// - A missing field reads as `null`, so [`Filter::IsNull`] matches an
    ///   absent field and [`ComparisonOp::Ne`] matches a document lacking it.
    /// - An ordering comparison matches only comparable types: a number with
    ///   a number, a string with a [`ScalarValue::Str`] or
    ///   [`ScalarValue::Timestamp`]. Equality treats incomparable types as
    ///   unequal.
    /// - [`Filter::Contains`], [`Filter::StartsWith`], and
    ///   [`Filter::EndsWith`] match only string fields.
    /// - [`Filter::ArrayContains`] matches only array fields, comparing each
    ///   element as equality does.
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
            Self::ArrayContains { field, value } => {
                lookup(doc, field).as_array().is_some_and(|elements| {
                    elements
                        .iter()
                        .any(|element| scalar_cmp(element, value) == Some(Ordering::Equal))
                })
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

fn next_iso_date(iso_date: &str) -> Result<String> {
    let (year, month, day) = parse_iso_date(iso_date)
        .ok_or_else(|| Error::server_error("invalid", format!("invalid ISO date: {iso_date:?}")))?;
    let (year, month, day) = if day < days_in_month(year, month) {
        (year, month, day + 1)
    } else if month < 12 {
        (year, month + 1, 1)
    } else if year < 9999 {
        (year + 1, 1, 1)
    } else {
        return Err(Error::server_error(
            "invalid",
            format!("cannot compute next day for date: {iso_date}"),
        ));
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

impl SortField {
    /// Sort by `field` from its least value to its greatest.
    #[must_use]
    pub fn asc(field: &str) -> Self {
        Self {
            field: field.to_owned(),
            descending: false,
        }
    }

    /// Sort by `field` from its greatest value to its least.
    #[must_use]
    pub fn desc(field: &str) -> Self {
        Self {
            field: field.to_owned(),
            descending: true,
        }
    }

    /// Order two JSON document bodies by this field.
    ///
    /// - The field path reads as in [`Filter::matches`].
    /// - Values of different JSON types order by type, `null` < bool < number
    ///   < string < array < object, so the order is total.
    /// - Two arrays, or two objects, are equal.
    /// - A descending field reverses the order.
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
///
/// [`Query::builder`] sets only the parts a query needs, as does
/// `..Query::default()`.
#[derive(Debug, Clone, Default, bon::Builder)]
pub struct Query {
    /// Optional filter tree.
    pub filter: Option<Filter>,

    /// Sort order (first key wins, then next, etc.).
    #[builder(default)]
    pub order_by: Vec<SortField>,

    /// Maximum documents to return.
    pub limit: Option<u32>,

    /// The `next` cursor of the previous [`Page`], or `None` for the first
    /// page.
    #[builder(into)]
    pub after: Option<String>,
}

/// The outcome of an [`insert`](Collection::insert) whose id is already stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Exists;

/// Newtype for timestamp strings so `Filter::gte("ts", Timestamp(...))` is explicit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Timestamp(pub String);
