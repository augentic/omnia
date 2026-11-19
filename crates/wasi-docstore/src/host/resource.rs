//! The values a backend exchanges with the host.

/// A stored document, as a backend reads and writes it.
#[derive(Debug, Clone)]
pub struct Document {
    /// The document's id.
    pub id: String,
    /// The body, a JSON object encoded as UTF-8.
    pub data: Vec<u8>,
}

/// A value a filter compares a field with.
#[derive(Debug, Clone)]
pub enum ScalarValue {
    /// JSON `null`.
    Null,
    /// A boolean.
    Boolean(bool),
    /// A whole number that fits in 32 bits.
    Int32(i32),
    /// A whole number.
    Int64(i64),
    /// A finite number.
    Float64(f64),
    /// Text.
    Str(String),
    /// Bytes.
    Binary(Vec<u8>),
    /// An RFC 3339 timestamp.
    Timestamp(String),
}

/// How a comparison relates a field to its value.
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

/// A field a query orders by.
#[derive(Debug, Clone)]
pub struct SortField {
    /// The field's path.
    pub field: String,
    /// Whether the field orders from greatest to least.
    pub descending: bool,
}

/// One page of a query, as a backend answers it.
#[derive(Debug, Clone)]
pub struct QueryResult {
    /// The documents on this page, in the order the query asked for.
    pub documents: Vec<Document>,
    /// The cursor that resumes the query, `None` on the last page.
    pub continuation: Option<String>,
}

/// A query's filter, as a backend evaluates it.
///
/// The host builds it from the guest's groups:
///
/// - a negated condition is a [`Not`](Self::Not);
/// - a group of several conditions is an [`Or`](Self::Or);
/// - several groups are an [`And`](Self::And).
#[derive(Debug, Clone)]
pub enum FilterTree {
    /// Field comparison.
    Compare {
        /// Field path.
        field: String,
        /// Comparison operator.
        op: ComparisonOp,
        /// Right-hand scalar.
        value: ScalarValue,
    },
    /// Field value is in the list.
    InList {
        /// Field path.
        field: String,
        /// Candidate values.
        values: Vec<ScalarValue>,
    },
    /// Field value is not in the list.
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
    /// String contains pattern.
    Contains {
        /// Field path.
        field: String,
        /// Pattern.
        pattern: String,
    },
    /// String starts with pattern.
    StartsWith {
        /// Field path.
        field: String,
        /// Pattern.
        pattern: String,
    },
    /// String ends with pattern.
    EndsWith {
        /// Field path.
        field: String,
        /// Pattern.
        pattern: String,
    },
    /// Logical AND.
    And(Vec<Self>),
    /// Logical OR.
    Or(Vec<Self>),
    /// Logical NOT.
    Not(Box<Self>),
}

impl FilterTree {
    /// Maximum nesting depth of the tree.
    pub fn depth(&self) -> usize {
        match self {
            Self::And(children) | Self::Or(children) => {
                1 + children.iter().map(Self::depth).max().unwrap_or(0)
            }
            Self::Not(inner) => 1 + inner.depth(),
            _ => 1,
        }
    }
}
