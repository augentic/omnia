use anyhow::Result;
use sea_query::{Alias, Expr, Order, SimpleExpr};

use crate::entity::values_to_wasi_datatypes;
use crate::filter::Filter;
use crate::join::{Join, JoinSpec};
use crate::query::{Query, QueryBuilder};

/// Builder for constructing SELECT queries.
pub struct SelectBuilder {
    table: String,
    columns: Vec<String>,
    aliases: Vec<(String, String, String)>,
    filters: Vec<SimpleExpr>,
    limit: Option<u64>,
    offset: Option<u64>,
    order: Vec<(String, Order)>,
    joins: Vec<JoinSpec>,
}

impl SelectBuilder {
    /// Creates a new SELECT query builder for the given table.
    #[must_use]
    pub fn new(table: &str) -> Self {
        Self {
            table: table.to_string(),
            columns: Vec::new(),
            aliases: Vec::new(),
            filters: Vec::new(),
            limit: None,
            offset: None,
            order: Vec::new(),
            joins: Vec::new(),
        }
    }

    /// Sets the columns to select.
    ///
    /// If neither `columns` nor `column_as` is called, the builder defaults to `SELECT *`.
    #[must_use]
    pub fn columns<I, S>(mut self, fields: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.columns = fields.into_iter().map(Into::into).collect();
        self
    }

    /// Adds an aliased column from a joined table.
    ///
    /// `source` is a `"table.column"` string, `alias` is the result column name.
    ///
    /// # Panics
    ///
    /// Panics if `source` does not contain a `.` separator.
    #[must_use]
    pub fn column_as(mut self, source: &str, alias: &str) -> Self {
        let (tbl, col) = source.split_once('.').unwrap_or_else(|| {
            panic!("column_as source must be \"table.column\", got \"{source}\"")
        });
        self.aliases.push((alias.to_string(), tbl.to_string(), col.to_string()));
        self
    }

    /// Adds a WHERE clause filter.
    #[must_use]
    pub fn filter(mut self, filter: Filter) -> Self {
        self.filters.push(filter.into_expr(&self.table));
        self
    }

    /// Sets the maximum number of rows to return.
    #[must_use]
    pub const fn limit(mut self, limit: u64) -> Self {
        self.limit = Some(limit);
        self
    }

    /// Sets the number of rows to skip.
    #[must_use]
    pub const fn offset(mut self, offset: u64) -> Self {
        self.offset = Some(offset);
        self
    }

    /// Adds an ORDER BY clause.
    #[must_use]
    pub fn order_by(mut self, column: &str, dir: Order) -> Self {
        self.order.push((column.to_string(), dir));
        self
    }

    /// Adds a JOIN clause to the query.
    #[must_use]
    pub fn join(mut self, join: Join) -> Self {
        self.joins.push(join.into_join_spec(&self.table));
        self
    }

    /// Build the SELECT query.
    ///
    /// # Errors
    ///
    /// Returns an error if query values cannot be converted to WASI data types.
    pub fn build(self) -> Result<Query> {
        let mut statement = sea_query::Query::select();

        if self.columns.is_empty() && self.aliases.is_empty() {
            statement.expr(Expr::cust("*"));
        } else {
            for field in &self.columns {
                statement.expr(Expr::cust(quoted_column(&self.table, field)));
            }
            for (alias, src_table, src_column) in &self.aliases {
                statement
                    .expr_as(Expr::cust(quoted_column(src_table, src_column)), Alias::new(alias));
            }
        }

        statement.from(Alias::new(&self.table));

        for JoinSpec { table, on, kind } in self.joins {
            statement.join(kind, Alias::new(table), on);
        }

        for filter in self.filters {
            statement.and_where(filter);
        }

        if let Some(limit) = self.limit {
            statement.limit(limit);
        }

        if let Some(offset) = self.offset {
            statement.offset(offset);
        }

        for (column, order) in self.order {
            statement.order_by_expr(Expr::cust(quoted_column(&self.table, &column)), order);
        }

        let (sql, values) = statement.build(QueryBuilder);
        let params = values_to_wasi_datatypes(values)?;

        tracing::debug!(
            table = %self.table,
            sql = %sql,
            param_count = params.len(),
            "SelectBuilder generated SQL"
        );

        Ok(Query { sql, params })
    }
}

/// Format a quoted `"table"."column"` reference for SQL.
#[must_use]
pub fn quoted_column(table: &str, column: &str) -> String {
    format!("\"{table}\".\"{column}\"")
}
