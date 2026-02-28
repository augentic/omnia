use anyhow::Result;
use sea_query::{Alias, SimpleExpr};

use crate::entity::values_to_wasi_datatypes;
use crate::filter::Filter;
use crate::query::{Query, QueryBuilder};

/// Builder for constructing DELETE queries.
pub struct DeleteBuilder {
    table: String,
    filters: Vec<SimpleExpr>,
}

impl DeleteBuilder {
    /// Creates a new DELETE query builder for the given table.
    #[must_use]
    pub fn new(table: &str) -> Self {
        Self {
            table: table.to_string(),
            filters: Vec::new(),
        }
    }

    /// Adds a WHERE clause filter.
    #[must_use]
    pub fn filter(mut self, filter: Filter) -> Self {
        self.filters.push(filter.into_expr(&self.table));
        self
    }

    /// Build the DELETE query.
    ///
    /// # Errors
    ///
    /// Returns an error if any query values cannot be converted to WASI data types.
    pub fn build(self) -> Result<Query> {
        let mut statement = sea_query::Query::delete();
        statement.from_table(Alias::new(&self.table));

        for filter in self.filters {
            statement.and_where(filter);
        }

        let (sql, values) = statement.build(QueryBuilder);
        let params = values_to_wasi_datatypes(values)?;

        tracing::debug!(
            table = %self.table,
            sql = %sql,
            param_count = params.len(),
            "DeleteBuilder generated SQL"
        );

        Ok(Query { sql, params })
    }
}
