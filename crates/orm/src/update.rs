use anyhow::Result;
use sea_query::{Alias, SimpleExpr, Value};

use crate::entity::values_to_wasi_datatypes;
use crate::filter::Filter;
use crate::query::{Query, QueryBuilder};

/// Builder for constructing UPDATE queries.
pub struct UpdateBuilder {
    table: String,
    set_clauses: Vec<(&'static str, Value)>,
    filters: Vec<SimpleExpr>,
}

impl UpdateBuilder {
    /// Creates a new UPDATE query builder for the given table.
    #[must_use]
    pub fn new(table: &str) -> Self {
        Self {
            table: table.to_string(),
            set_clauses: Vec::new(),
            filters: Vec::new(),
        }
    }

    /// Sets a column to a new value.
    #[must_use]
    pub fn set<V>(mut self, column: &'static str, value: V) -> Self
    where
        V: Into<Value>,
    {
        self.set_clauses.push((column, value.into()));
        self
    }

    /// Conditionally sets a column when the value is `Some`.
    ///
    /// This is useful for partial updates where only provided fields should be modified.
    #[must_use]
    pub fn set_if<V>(self, column: &'static str, value: Option<V>) -> Self
    where
        V: Into<Value>,
    {
        match value {
            Some(v) => self.set(column, v),
            None => self,
        }
    }

    /// Adds a WHERE clause filter.
    #[must_use]
    pub fn filter(mut self, filter: Filter) -> Self {
        self.filters.push(filter.into_expr(&self.table));
        self
    }

    /// Build the UPDATE query.
    ///
    /// # Errors
    ///
    /// Returns an error if query values cannot be converted to WASI data types.
    pub fn build(self) -> Result<Query> {
        let mut statement = sea_query::Query::update();
        statement.table(Alias::new(&self.table));

        for (column, value) in self.set_clauses {
            statement.value(Alias::new(column), value);
        }

        for expr in self.filters {
            statement.and_where(expr);
        }

        let (sql, values) = statement.build(QueryBuilder);
        let params = values_to_wasi_datatypes(values)?;

        tracing::debug!(
            table = %self.table,
            sql = %sql,
            param_count = params.len(),
            "UpdateBuilder generated SQL"
        );

        Ok(Query { sql, params })
    }
}
