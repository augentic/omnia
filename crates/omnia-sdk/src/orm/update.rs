use std::marker::PhantomData;

use anyhow::Result;
use sea_query::{Alias, SimpleExpr, Value};

use super::entity::Entity;
use super::filter::Filter;
use super::query::{Query, finish};

/// Builder for constructing UPDATE queries.
pub struct UpdateBuilder<M: Entity> {
    set_clauses: Vec<(&'static str, Value)>,
    filters: Vec<SimpleExpr>,
    returning: Vec<&'static str>,
    _marker: PhantomData<M>,
}

impl<M: Entity> Default for UpdateBuilder<M> {
    fn default() -> Self {
        Self {
            set_clauses: Vec::new(),
            filters: Vec::new(),
            returning: Vec::new(),
            _marker: PhantomData,
        }
    }
}

impl<M: Entity> UpdateBuilder<M> {
    /// Creates a new UPDATE query builder.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
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

    /// Adds a WHERE clause filter.
    #[must_use]
    pub fn r#where(mut self, filter: Filter) -> Self {
        self.filters.push(filter.into_expr(M::TABLE));
        self
    }

    /// Specifies columns to return from updated rows.
    #[must_use]
    pub fn returning(mut self, column: &'static str) -> Self {
        self.returning.push(column);
        self
    }

    /// Build the UPDATE query.
    ///
    /// # Errors
    ///
    /// Returns an error if query values cannot be converted to WASI data types.
    pub fn build(self) -> Result<Query> {
        let mut statement = sea_query::Query::update();
        statement.table(Alias::new(M::TABLE));

        for (column, value) in self.set_clauses {
            statement.value(Alias::new(column), value);
        }

        for expr in self.filters {
            statement.and_where(expr);
        }

        for column in self.returning {
            statement.returning_col(Alias::new(column));
        }

        finish(&statement, M::TABLE, "update")
    }
}
