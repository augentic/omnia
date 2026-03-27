//! Filter tree and resource proxy for `wasi:jsondb` host bindings.

use crate::host::generated::wasi::jsondb::types::{ComparisonOp, ScalarValue};

/// Internal recursive filter representation built from WIT `filter` resource constructors.
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

/// Wrapper stored in the wasmtime resource table for `filter` handles.
#[derive(Debug, Clone)]
pub struct FilterProxy(pub FilterTree);
