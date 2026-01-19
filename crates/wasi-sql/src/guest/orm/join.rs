use sea_query::{JoinType, SimpleExpr};

use crate::orm::filter::Filter;

/// Represents a SQL join operation without exposing ``SeaQuery`` types to guest code.
#[derive(Clone)]
pub struct Join {
    table: &'static str,
    alias: Option<&'static str>,
    on: Filter,
    kind: JoinKind,
}

/// Join types supported by the ORM.
#[derive(Clone, Copy)]
pub enum JoinKind {
    Inner,
    Left,
    Right,
    Full,
}

impl Join {
    /// Creates a JOIN (defaults to INNER JOIN).
    #[must_use]
    pub const fn new(table: &'static str, on: Filter) -> Self {
        Self {
            table,
            alias: None,
            on,
            kind: JoinKind::Inner,
        }
    }

    /// Creates a LEFT JOIN.
    #[must_use]
    pub const fn left(table: &'static str, on: Filter) -> Self {
        Self {
            table,
            alias: None,
            on,
            kind: JoinKind::Left,
        }
    }

    /// Creates a RIGHT JOIN.
    #[must_use]
    pub const fn right(table: &'static str, on: Filter) -> Self {
        Self {
            table,
            alias: None,
            on,
            kind: JoinKind::Right,
        }
    }

    /// Creates a FULL OUTER JOIN.
    #[must_use]
    pub const fn full(table: &'static str, on: Filter) -> Self {
        Self {
            table,
            alias: None,
            on,
            kind: JoinKind::Full,
        }
    }

    /// Creates an INNER JOIN (alias for `new`).
    #[must_use]
    pub const fn inner(table: &'static str, on: Filter) -> Self {
        Self::new(table, on)
    }

    /// Sets an alias for the joined table.
    #[must_use]
    pub const fn alias(mut self, alias: &'static str) -> Self {
        self.alias = Some(alias);
        self
    }

    /// Converts this Join into a ``JoinSpec`` for ``SeaQuery``.
    /// The ``default_table`` is the primary table being selected from.
    pub(crate) fn into_join_spec(self, default_table: &'static str) -> JoinSpec {
        JoinSpec {
            table: self.table,
            alias: self.alias,
            on: self.on.into_expr(default_table),
            kind: self.kind.into_join_type(),
        }
    }
}

impl JoinKind {
    const fn into_join_type(self) -> JoinType {
        match self {
            Self::Inner => JoinType::InnerJoin,
            Self::Left => JoinType::LeftJoin,
            Self::Right => JoinType::RightJoin,
            Self::Full => JoinType::FullOuterJoin,
        }
    }
}

/// Internal representation used by ``SeaQuery``.
/// This is kept internal to the ORM layer.
#[derive(Clone)]
pub struct JoinSpec {
    pub table: &'static str,
    pub alias: Option<&'static str>,
    pub on: SimpleExpr,
    pub kind: JoinType,
}
