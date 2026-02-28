use sea_query::{Expr, ExprTrait, SimpleExpr, Value};

use crate::select::quoted_column;

/// Filter represents database predicates without exposing `SeaQuery` types to guest code.
///
/// Values are stored internally as `sea_query::Value` but guest code never imports `SeaQuery`.
/// Instead, guests use natural Rust types (i32, String, `DateTime<Utc>`) which convert via From.
///
/// For filters with optional table parameter: None uses the entity's default table,
/// `Some("table_name")` uses the specified table (useful for joins via `.on()`).
#[derive(Debug, Clone)]
pub enum Filter {
    /// [table.]column = value
    Eq(Option<&'static str>, &'static str, Value),
    /// [table.]column > value
    Gt(Option<&'static str>, &'static str, Value),
    /// [table.]column < value
    Lt(Option<&'static str>, &'static str, Value),
    /// [table.]column IN (values)
    In(Option<&'static str>, &'static str, Vec<Value>),
    /// [table.]column IS NULL
    IsNull(Option<&'static str>, &'static str),
    /// [table.]column IS NOT NULL
    IsNotNull(Option<&'static str>, &'static str),
    /// [table.]column LIKE pattern
    Like(Option<&'static str>, &'static str, String),
    /// Column-to-column comparison: table1.col1 = table2.col2
    ColEq(&'static str, &'static str, &'static str, &'static str),
    /// Logical AND of multiple filters
    And(Vec<Self>),
    /// Logical OR of multiple filters
    Or(Vec<Self>),
    /// Logical NOT of a filter
    Not(Box<Self>),
}

impl Filter {
    fn resolve_column(
        tbl: Option<&'static str>, col: &'static str, default_table: &str,
    ) -> SimpleExpr {
        Expr::cust(quoted_column(tbl.unwrap_or(default_table), col))
    }

    /// Convert Filter to `SeaQuery` `SimpleExpr` using the specified table name.
    #[must_use]
    pub fn into_expr(self, default_table: &str) -> SimpleExpr {
        match self {
            Self::Eq(tbl, col, val) => Self::resolve_column(tbl, col, default_table).eq(val),
            Self::Gt(tbl, col, val) => Self::resolve_column(tbl, col, default_table).gt(val),
            Self::Lt(tbl, col, val) => Self::resolve_column(tbl, col, default_table).lt(val),
            Self::In(tbl, col, vals) => Self::resolve_column(tbl, col, default_table).is_in(vals),
            Self::IsNull(tbl, col) => Self::resolve_column(tbl, col, default_table).is_null(),
            Self::IsNotNull(tbl, col) => {
                Self::resolve_column(tbl, col, default_table).is_not_null()
            }
            Self::Like(tbl, col, pattern) => {
                Self::resolve_column(tbl, col, default_table).like(pattern)
            }
            Self::ColEq(tbl1, col1, tbl2, col2) => {
                Expr::cust(quoted_column(tbl1, col1)).eq(Expr::cust(quoted_column(tbl2, col2)))
            }
            Self::And(filters) => {
                let mut exprs = filters.into_iter().map(|f| f.into_expr(default_table));
                exprs.next().map_or_else(
                    || Expr::value(true),
                    |first| exprs.fold(first, sea_query::SimpleExpr::and),
                )
            }
            Self::Or(filters) => {
                let mut exprs = filters.into_iter().map(|f| f.into_expr(default_table));
                exprs.next().map_or_else(
                    || Expr::value(false),
                    |first| exprs.fold(first, sea_query::SimpleExpr::or),
                )
            }
            Self::Not(filter) => Expr::expr(filter.into_expr(default_table)).not(),
        }
    }

    /// Sets a table qualifier on this filter.
    ///
    /// Use this instead of separate `table_eq()`, `table_gt()`, etc. constructors.
    ///
    /// # Example
    ///
    /// ```ignore
    /// Filter::eq("name", "ACME").on("agency")
    /// ```
    #[must_use]
    pub fn on(self, table: &'static str) -> Self {
        match self {
            Self::Eq(_, col, val) => Self::Eq(Some(table), col, val),
            Self::Gt(_, col, val) => Self::Gt(Some(table), col, val),
            Self::Lt(_, col, val) => Self::Lt(Some(table), col, val),
            Self::In(_, col, vals) => Self::In(Some(table), col, vals),
            Self::IsNull(_, col) => Self::IsNull(Some(table), col),
            Self::IsNotNull(_, col) => Self::IsNotNull(Some(table), col),
            Self::Like(_, col, pattern) => Self::Like(Some(table), col, pattern),
            other => other,
        }
    }

    /// Creates an equality filter (column = value).
    #[must_use]
    pub fn eq(col: &'static str, val: impl Into<Value>) -> Self {
        Self::Eq(None, col, val.into())
    }

    /// Creates a greater-than filter (column > value).
    #[must_use]
    pub fn gt(col: &'static str, val: impl Into<Value>) -> Self {
        Self::Gt(None, col, val.into())
    }

    /// Creates a less-than filter (column < value).
    #[must_use]
    pub fn lt(col: &'static str, val: impl Into<Value>) -> Self {
        Self::Lt(None, col, val.into())
    }

    /// Creates an IN filter (column IN (values)).
    #[must_use]
    pub fn r#in(col: &'static str, vals: impl IntoIterator<Item = impl Into<Value>>) -> Self {
        Self::In(None, col, vals.into_iter().map(Into::into).collect())
    }

    /// Creates an IS NULL filter.
    #[must_use]
    pub const fn is_null(col: &'static str) -> Self {
        Self::IsNull(None, col)
    }

    /// Creates an IS NOT NULL filter.
    #[must_use]
    pub const fn is_not_null(col: &'static str) -> Self {
        Self::IsNotNull(None, col)
    }

    /// Creates a LIKE filter with pattern matching.
    #[must_use]
    pub fn like(col: &'static str, pattern: impl Into<String>) -> Self {
        Self::Like(None, col, pattern.into())
    }

    /// Compare two columns for equality across tables.
    #[must_use]
    pub const fn col_eq(
        table1: &'static str, col1: &'static str, table2: &'static str, col2: &'static str,
    ) -> Self {
        Self::ColEq(table1, col1, table2, col2)
    }
}
