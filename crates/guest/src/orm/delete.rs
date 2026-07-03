use std::marker::PhantomData;

use anyhow::Result;
use sea_query::{Alias, SimpleExpr};

use super::entity::Entity;
use super::filter::Filter;
use super::query::{Query, finish};

/// Builder for constructing DELETE queries.
pub struct DeleteBuilder<M: Entity> {
    filters: Vec<SimpleExpr>,
    returning: Vec<&'static str>,
    _marker: PhantomData<M>,
}

impl<M: Entity> Default for DeleteBuilder<M> {
    fn default() -> Self {
        Self {
            filters: Vec::new(),
            returning: Vec::new(),
            _marker: PhantomData,
        }
    }
}

impl<M: Entity> DeleteBuilder<M> {
    /// Creates a new DELETE query builder.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a WHERE clause filter.
    #[must_use]
    pub fn r#where(mut self, filter: Filter) -> Self {
        self.filters.push(filter.into_expr(M::TABLE));
        self
    }

    /// Specifies columns to return from deleted rows.
    #[must_use]
    pub fn returning(mut self, column: &'static str) -> Self {
        self.returning.push(column);
        self
    }

    /// Build the DELETE query.
    ///
    /// # Errors
    ///
    /// Returns an error if no `WHERE` filter was set (an unfiltered DELETE would
    /// remove every row), or if a query value cannot be converted to a WASI data
    /// type.
    pub fn build(self) -> Result<Query> {
        if self.filters.is_empty() {
            anyhow::bail!("refusing to build an unfiltered DELETE; add a `.where(...)` clause");
        }

        let mut statement = sea_query::Query::delete();
        statement.from_table(Alias::new(M::TABLE));

        for filter in self.filters {
            statement.and_where(filter);
        }

        for column in self.returning {
            statement.returning_col(Alias::new(column));
        }

        finish(&statement, M::TABLE, "delete")
    }
}
