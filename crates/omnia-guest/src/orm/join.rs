use sea_query::{JoinType, SimpleExpr};

use super::filter::Filter;

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
    /// `INNER JOIN`
    Inner,
    /// `LEFT JOIN`
    Left,
    /// `RIGHT JOIN`
    Right,
    /// `FULL OUTER JOIN`
    Full,
}

impl Join {
    /// Creates a JOIN of the given kind.
    #[must_use]
    pub const fn new(table: &'static str, kind: JoinKind, on: Filter) -> Self {
        Self {
            table,
            alias: None,
            on,
            kind,
        }
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

macro_rules! join_ctor {
    ($name:ident, $kind:ident, $doc:literal) => {
        #[doc = $doc]
        #[must_use]
        pub const fn $name(table: &'static str, on: Filter) -> Self {
            Self::new(table, JoinKind::$kind, on)
        }
    };
}

impl Join {
    join_ctor!(inner, Inner, "Creates an INNER JOIN.");

    join_ctor!(left, Left, "Creates a LEFT JOIN.");

    join_ctor!(right, Right, "Creates a RIGHT JOIN.");

    join_ctor!(full, Full, "Creates a FULL OUTER JOIN.");
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
