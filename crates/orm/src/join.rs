use sea_query::{JoinType, SimpleExpr};

use crate::filter::Filter;

/// Represents a SQL join operation without exposing `SeaQuery` types to guest code.
#[derive(Clone)]
pub struct Join {
    table: &'static str,
    on: Filter,
    kind: JoinType,
}

impl Join {
    /// Creates an INNER JOIN.
    #[must_use]
    pub const fn inner(table: &'static str, on: Filter) -> Self {
        Self {
            table,
            on,
            kind: JoinType::InnerJoin,
        }
    }

    /// Creates a LEFT JOIN.
    #[must_use]
    pub const fn left(table: &'static str, on: Filter) -> Self {
        Self {
            table,
            on,
            kind: JoinType::LeftJoin,
        }
    }

    /// Creates a RIGHT JOIN.
    #[must_use]
    pub const fn right(table: &'static str, on: Filter) -> Self {
        Self {
            table,
            on,
            kind: JoinType::RightJoin,
        }
    }

    /// Creates a FULL OUTER JOIN.
    #[must_use]
    pub const fn full(table: &'static str, on: Filter) -> Self {
        Self {
            table,
            on,
            kind: JoinType::FullOuterJoin,
        }
    }

    /// Converts this Join into a `JoinSpec` for `SeaQuery`.
    pub(crate) fn into_join_spec(self, default_table: &str) -> JoinSpec {
        JoinSpec {
            table: self.table,
            on: self.on.into_expr(default_table),
            kind: self.kind,
        }
    }
}

/// Internal representation used by `SeaQuery`.
#[derive(Clone)]
pub struct JoinSpec {
    pub table: &'static str,
    pub on: SimpleExpr,
    pub kind: JoinType,
}
