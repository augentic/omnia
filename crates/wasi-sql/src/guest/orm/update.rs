use std::marker::PhantomData;

use anyhow::Result;
use sea_query::{Alias, Query, SimpleExpr, Value};

use crate::orm::entity::{Entity, values_to_wasi_datatypes};
use crate::orm::filter::Filter;
use crate::orm::query::{BuiltQuery, OrmQueryBuilder};

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
    pub fn build(self) -> Result<BuiltQuery> {
        let mut statement = Query::update();
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

        let (sql, values) = statement.build(OrmQueryBuilder::default());
        let params = values_to_wasi_datatypes(values)?;

        tracing::debug!(
            table = M::TABLE,
            sql = %sql,
            param_count = params.len(),
            "UpdateBuilder generated SQL"
        );

        Ok(BuiltQuery { sql, params })
    }
}
