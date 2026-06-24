use sea_query::{Expr, ExprTrait, SimpleExpr, Value};

use super::select::table_column;

/// A column reference, optionally qualified with a table name.
///
/// When `table` is `None`, the column resolves against the entity's default table
/// at query-build time. Use [`ColRef::qualified`] to bind a specific table for
/// joined queries.
#[derive(Debug, Clone, Copy)]
pub struct ColRef {
    /// Optional table qualifier. When `None`, the entity's default table is used at build time.
    pub table: Option<&'static str>,
    /// Column name.
    pub column: &'static str,
}

impl ColRef {
    /// Constructs a column reference with no table qualifier (resolved against the entity's
    /// default table at build time).
    #[must_use]
    pub const fn unqualified(column: &'static str) -> Self {
        Self { table: None, column }
    }

    /// Constructs an explicitly table-qualified column reference.
    #[must_use]
    pub const fn qualified(table: &'static str, column: &'static str) -> Self {
        Self {
            table: Some(table),
            column,
        }
    }

    fn resolve(self, default_table: &'static str) -> SimpleExpr {
        Expr::col(table_column(self.table.unwrap_or(default_table), self.column))
    }
}

/// Comparison operators for column predicates.
#[derive(Debug, Clone, Copy)]
pub enum CmpOp {
    /// `=`
    Eq,
    /// `!=`
    Ne,
    /// `>`
    Gt,
    /// `>=`
    Gte,
    /// `<`
    Lt,
    /// `<=`
    Lte,
}

/// Filter represents database predicates without exposing ``SeaQuery`` types to guest code.
///
/// Values are stored internally as ``sea_query::Value`` but guest code never imports ``SeaQuery``.
/// Instead, guests use natural Rust types (i32, String, ``DateTime<Utc>``) which convert via From.
/// Build filters with the associated functions (e.g. [`Filter::eq`]); the predicate shape is
/// intentionally private so the internal representation can evolve.
#[derive(Debug, Clone)]
pub struct Filter(FilterKind);

#[derive(Debug, Clone)]
enum FilterKind {
    /// `col <op> value`
    Compare(ColRef, CmpOp, Value),
    /// `col IN (values)` or `col NOT IN (values)` when negated.
    In(ColRef, Vec<Value>, bool),
    /// `col IS NULL` or `col IS NOT NULL` when negated.
    Null(ColRef, bool),
    /// `col LIKE pattern` or `col NOT LIKE pattern` when negated.
    Like(ColRef, String, bool),
    /// `col BETWEEN low AND high` or `col NOT BETWEEN low AND high` when negated.
    Between(ColRef, Value, Value, bool),
    /// Column-to-column comparison, e.g. `table1.col1 <op> table2.col2`.
    ColCompare(ColRef, CmpOp, ColRef),
    /// Logical AND of multiple filters.
    And(Vec<Filter>),
    /// Logical OR of multiple filters.
    Or(Vec<Filter>),
    /// Logical NOT of a filter.
    Not(Box<Filter>),
}

fn apply_cmp(left: SimpleExpr, op: CmpOp, right: SimpleExpr) -> SimpleExpr {
    match op {
        CmpOp::Eq => left.eq(right),
        CmpOp::Ne => left.ne(right),
        CmpOp::Gt => left.gt(right),
        CmpOp::Gte => left.gte(right),
        CmpOp::Lt => left.lt(right),
        CmpOp::Lte => left.lte(right),
    }
}

impl Filter {
    /// Convert Filter to ``SeaQuery`` ``SimpleExpr`` using the specified default table.
    #[must_use]
    pub(crate) fn into_expr(self, default_table: &'static str) -> SimpleExpr {
        match self.0 {
            FilterKind::Compare(col, op, val) => {
                apply_cmp(col.resolve(default_table), op, val.into())
            }
            FilterKind::In(col, vals, false) => col.resolve(default_table).is_in(vals),
            FilterKind::In(col, vals, true) => col.resolve(default_table).is_not_in(vals),
            FilterKind::Null(col, false) => col.resolve(default_table).is_null(),
            FilterKind::Null(col, true) => col.resolve(default_table).is_not_null(),
            FilterKind::Like(col, pattern, false) => col.resolve(default_table).like(pattern),
            FilterKind::Like(col, pattern, true) => col.resolve(default_table).not_like(pattern),
            FilterKind::Between(col, low, high, false) => {
                col.resolve(default_table).between(low, high)
            }
            FilterKind::Between(col, low, high, true) => {
                col.resolve(default_table).not_between(low, high)
            }
            FilterKind::ColCompare(left, op, right) => {
                apply_cmp(left.resolve(default_table), op, right.resolve(default_table))
            }
            FilterKind::And(filters) => {
                let mut exprs = filters.into_iter().map(|f| f.into_expr(default_table));
                exprs.next().map_or_else(
                    || Expr::value(true),
                    |first| exprs.fold(first, sea_query::SimpleExpr::and),
                )
            }
            FilterKind::Or(filters) => {
                let mut exprs = filters.into_iter().map(|f| f.into_expr(default_table));
                exprs.next().map_or_else(
                    || Expr::value(false),
                    |first| exprs.fold(first, sea_query::SimpleExpr::or),
                )
            }
            FilterKind::Not(filter) => Expr::expr(filter.into_expr(default_table)).not(),
        }
    }

    /// Qualifies an existing filter with a specific table name.
    ///
    /// Applies recursively to nested combinators (`And`/`Or`/`Not`). For column-to-column
    /// comparisons (`ColCompare`) the table qualifiers are already explicit and this is a no-op.
    #[must_use]
    pub fn in_table(self, table: &'static str) -> Self {
        let set = |col: ColRef| ColRef {
            table: Some(table),
            ..col
        };
        Self(match self.0 {
            FilterKind::Compare(col, op, v) => FilterKind::Compare(set(col), op, v),
            FilterKind::In(col, vals, neg) => FilterKind::In(set(col), vals, neg),
            FilterKind::Null(col, neg) => FilterKind::Null(set(col), neg),
            FilterKind::Like(col, pat, neg) => FilterKind::Like(set(col), pat, neg),
            FilterKind::Between(col, lo, hi, neg) => FilterKind::Between(set(col), lo, hi, neg),
            FilterKind::And(filters) => {
                FilterKind::And(filters.into_iter().map(|f| f.in_table(table)).collect())
            }
            FilterKind::Or(filters) => {
                FilterKind::Or(filters.into_iter().map(|f| f.in_table(table)).collect())
            }
            FilterKind::Not(inner) => FilterKind::Not(Box::new(inner.in_table(table))),
            other @ FilterKind::ColCompare(..) => other,
        })
    }
}

macro_rules! cmp_ctor {
    ($name:ident, $op:ident, $doc:literal) => {
        #[doc = $doc]
        #[must_use]
        pub fn $name(col: &'static str, val: impl Into<Value>) -> Self {
            Self(FilterKind::Compare(ColRef::unqualified(col), CmpOp::$op, val.into()))
        }
    };
}

macro_rules! table_cmp_ctor {
    ($name:ident, $op:ident, $doc:literal) => {
        #[doc = $doc]
        #[must_use]
        pub fn $name(table: &'static str, col: &'static str, val: impl Into<Value>) -> Self {
            Self(FilterKind::Compare(ColRef::qualified(table, col), CmpOp::$op, val.into()))
        }
    };
}

macro_rules! list_ctor {
    ($name:ident, $negated:literal, $doc:literal) => {
        #[doc = $doc]
        #[must_use]
        pub fn $name(col: &'static str, vals: impl IntoIterator<Item = impl Into<Value>>) -> Self {
            Self(FilterKind::In(
                ColRef::unqualified(col),
                vals.into_iter().map(Into::into).collect(),
                $negated,
            ))
        }
    };
}

macro_rules! table_list_ctor {
    ($name:ident, $negated:literal, $doc:literal) => {
        #[doc = $doc]
        #[must_use]
        pub fn $name(
            table: &'static str, col: &'static str,
            vals: impl IntoIterator<Item = impl Into<Value>>,
        ) -> Self {
            Self(FilterKind::In(
                ColRef::qualified(table, col),
                vals.into_iter().map(Into::into).collect(),
                $negated,
            ))
        }
    };
}

impl Filter {
    cmp_ctor!(eq, Eq, "Creates an equality filter (column = value).");

    cmp_ctor!(ne, Ne, "Creates an inequality filter (column != value).");

    cmp_ctor!(gt, Gt, "Creates a greater-than filter (column > value).");

    cmp_ctor!(gte, Gte, "Creates a greater-than-or-equal filter (column >= value).");

    cmp_ctor!(lt, Lt, "Creates a less-than filter (column < value).");

    cmp_ctor!(lte, Lte, "Creates a less-than-or-equal filter (column <= value).");

    list_ctor!(r#in, false, "Creates an IN filter (column IN (values)).");

    list_ctor!(not_in, true, "Creates a NOT IN filter (column NOT IN (values)).");

    table_cmp_ctor!(
        table_eq,
        Eq,
        "Creates a table-qualified equality filter (table.column = value)."
    );

    table_cmp_ctor!(
        table_ne,
        Ne,
        "Creates a table-qualified inequality filter (table.column != value)."
    );

    table_cmp_ctor!(
        table_gt,
        Gt,
        "Creates a table-qualified greater-than filter (table.column > value)."
    );

    table_cmp_ctor!(
        table_gte,
        Gte,
        "Creates a table-qualified greater-than-or-equal filter (table.column >= value)."
    );

    table_cmp_ctor!(
        table_lt,
        Lt,
        "Creates a table-qualified less-than filter (table.column < value)."
    );

    table_cmp_ctor!(
        table_lte,
        Lte,
        "Creates a table-qualified less-than-or-equal filter (table.column <= value)."
    );

    table_list_ctor!(
        table_in,
        false,
        "Creates a table-qualified IN filter (table.column IN (values))."
    );

    table_list_ctor!(
        table_not_in,
        true,
        "Creates a table-qualified NOT IN filter (table.column NOT IN (values))."
    );

    /// Creates an IS NULL filter.
    #[must_use]
    pub const fn is_null(col: &'static str) -> Self {
        Self(FilterKind::Null(ColRef::unqualified(col), false))
    }

    /// Creates an IS NOT NULL filter.
    #[must_use]
    pub const fn is_not_null(col: &'static str) -> Self {
        Self(FilterKind::Null(ColRef::unqualified(col), true))
    }

    /// Creates a LIKE filter with pattern matching.
    #[must_use]
    pub const fn like(col: &'static str, pattern: String) -> Self {
        Self(FilterKind::Like(ColRef::unqualified(col), pattern, false))
    }

    /// Creates a NOT LIKE filter with pattern matching.
    #[must_use]
    pub const fn not_like(col: &'static str, pattern: String) -> Self {
        Self(FilterKind::Like(ColRef::unqualified(col), pattern, true))
    }

    /// Creates a BETWEEN filter (column BETWEEN low AND high).
    #[must_use]
    pub fn between(col: &'static str, low: impl Into<Value>, high: impl Into<Value>) -> Self {
        Self(FilterKind::Between(ColRef::unqualified(col), low.into(), high.into(), false))
    }

    /// Creates a NOT BETWEEN filter.
    #[must_use]
    pub fn not_between(col: &'static str, low: impl Into<Value>, high: impl Into<Value>) -> Self {
        Self(FilterKind::Between(ColRef::unqualified(col), low.into(), high.into(), true))
    }

    /// Combines filters with logical AND. Empty list evaluates to `true`.
    #[must_use]
    pub fn and(filters: impl IntoIterator<Item = Self>) -> Self {
        Self(FilterKind::And(filters.into_iter().collect()))
    }

    /// Combines filters with logical OR. Empty list evaluates to `false`.
    #[must_use]
    pub fn or(filters: impl IntoIterator<Item = Self>) -> Self {
        Self(FilterKind::Or(filters.into_iter().collect()))
    }

    /// Logically negates a filter.
    #[must_use]
    #[allow(clippy::should_implement_trait)]
    pub fn not(filter: Self) -> Self {
        Self(FilterKind::Not(Box::new(filter)))
    }

    /// Creates a table-qualified IS NULL filter (table.column IS NULL).
    #[must_use]
    pub const fn table_is_null(table: &'static str, col: &'static str) -> Self {
        Self(FilterKind::Null(ColRef::qualified(table, col), false))
    }

    /// Creates a table-qualified IS NOT NULL filter (table.column IS NOT NULL).
    #[must_use]
    pub const fn table_is_not_null(table: &'static str, col: &'static str) -> Self {
        Self(FilterKind::Null(ColRef::qualified(table, col), true))
    }

    /// Creates a table-qualified LIKE filter (table.column LIKE pattern).
    #[must_use]
    pub const fn table_like(table: &'static str, col: &'static str, pattern: String) -> Self {
        Self(FilterKind::Like(ColRef::qualified(table, col), pattern, false))
    }

    /// Creates a table-qualified NOT LIKE filter (table.column NOT LIKE pattern).
    #[must_use]
    pub const fn table_not_like(table: &'static str, col: &'static str, pattern: String) -> Self {
        Self(FilterKind::Like(ColRef::qualified(table, col), pattern, true))
    }

    /// Creates a table-qualified BETWEEN filter (table.column BETWEEN low AND high).
    #[must_use]
    pub fn table_between(
        table: &'static str, col: &'static str, low: impl Into<Value>, high: impl Into<Value>,
    ) -> Self {
        Self(FilterKind::Between(ColRef::qualified(table, col), low.into(), high.into(), false))
    }

    /// Creates a table-qualified NOT BETWEEN filter.
    #[must_use]
    pub fn table_not_between(
        table: &'static str, col: &'static str, low: impl Into<Value>, high: impl Into<Value>,
    ) -> Self {
        Self(FilterKind::Between(ColRef::qualified(table, col), low.into(), high.into(), true))
    }

    /// Compares two columns for equality (`table1.col1 = table2.col2`).
    /// Used primarily in JOIN conditions.
    #[must_use]
    pub const fn col_eq(
        table1: &'static str, col1: &'static str, table2: &'static str, col2: &'static str,
    ) -> Self {
        Self(FilterKind::ColCompare(
            ColRef::qualified(table1, col1),
            CmpOp::Eq,
            ColRef::qualified(table2, col2),
        ))
    }
}
