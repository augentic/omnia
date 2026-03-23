//! Domain types for JSON document storage and queries (shared by host and guest).

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

/// Sort field for queries.
#[derive(Debug, Clone, Default)]
pub struct SortField {
    /// Field path.
    pub field: String,
    /// When `true`, sort descending.
    pub descending: bool,
}

/// Options for listing or searching documents.
#[derive(Debug, Clone, Default)]
pub struct QueryOptions {
    /// Optional filter tree.
    pub filter: Option<Filter>,
    /// Sort order (first key wins, then next, etc.).
    pub order_by: Vec<SortField>,
    /// Maximum documents to return.
    pub limit: Option<u32>,
    /// Skip this many documents after filter/sort (offset pagination).
    pub offset: Option<u32>,
    /// Opaque continuation token from a previous page.
    pub continuation: Option<String>,
}

/// Stored document: identifier plus JSON body bytes.
#[derive(Debug, Clone)]
pub struct Document {
    /// Primary key string.
    pub id: String,
    /// JSON payload.
    pub data: Vec<u8>,
}

/// Result of a query with optional next-page token.
#[derive(Debug, Clone, Default)]
pub struct QueryResult {
    /// Matching documents.
    pub documents: Vec<Document>,
    /// Continuation token for the next page, if any.
    pub continuation: Option<String>,
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

/// Newtype for timestamp strings so `Filter::gte("ts", Timestamp(...))` is explicit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Timestamp(pub String);

impl From<Timestamp> for ScalarValue {
    fn from(t: Timestamp) -> Self {
        Self::Timestamp(t.0)
    }
}

impl Filter {
    /// Equality comparison.
    #[must_use]
    pub fn eq(field: &str, val: impl Into<ScalarValue>) -> Self {
        Self::Compare {
            field: field.to_string(),
            op: ComparisonOp::Eq,
            value: val.into(),
        }
    }

    /// Inequality comparison.
    #[must_use]
    pub fn ne(field: &str, val: impl Into<ScalarValue>) -> Self {
        Self::Compare {
            field: field.to_string(),
            op: ComparisonOp::Ne,
            value: val.into(),
        }
    }

    /// Greater than.
    #[must_use]
    pub fn gt(field: &str, val: impl Into<ScalarValue>) -> Self {
        Self::Compare {
            field: field.to_string(),
            op: ComparisonOp::Gt,
            value: val.into(),
        }
    }

    /// Greater than or equal.
    #[must_use]
    pub fn gte(field: &str, val: impl Into<ScalarValue>) -> Self {
        Self::Compare {
            field: field.to_string(),
            op: ComparisonOp::Gte,
            value: val.into(),
        }
    }

    /// Less than.
    #[must_use]
    pub fn lt(field: &str, val: impl Into<ScalarValue>) -> Self {
        Self::Compare {
            field: field.to_string(),
            op: ComparisonOp::Lt,
            value: val.into(),
        }
    }

    /// Less than or equal.
    #[must_use]
    pub fn lte(field: &str, val: impl Into<ScalarValue>) -> Self {
        Self::Compare {
            field: field.to_string(),
            op: ComparisonOp::Lte,
            value: val.into(),
        }
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
    #[allow(clippy::should_implement_trait)] // Domain API mirrors `std::ops::Not`, not the trait.
    pub fn not(inner: Self) -> Self {
        Self::Not(Box::new(inner))
    }

    /// Restrict `field` to a calendar date (UTC day) using range on timestamp strings.
    #[must_use]
    pub fn on_date(field: &str, iso_date: &str) -> Self {
        let start = format!("{iso_date}T00:00:00Z");
        let end_date = next_iso_date(iso_date);
        let end = format!("{end_date}T00:00:00Z");
        Self::And(vec![Self::gte(field, Timestamp(start)), Self::lt(field, Timestamp(end))])
    }
}

/// Advance `YYYY-MM-DD` by one day (UTC). Falls back to `iso_date` on parse failure.
fn next_iso_date(iso_date: &str) -> String {
    use chrono::NaiveDate;
    NaiveDate::parse_from_str(iso_date, "%Y-%m-%d").map_or_else(
        |_| iso_date.to_string(),
        |d| d.succ_opt().map_or_else(|| iso_date.to_string(), |n| n.format("%Y-%m-%d").to_string()),
    )
}
