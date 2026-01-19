use std::marker::PhantomData;

use anyhow::Result;
use sea_query::{Alias, Query, SimpleExpr};

use crate::orm::entity::{Entity, values_to_wasi_datatypes};
use crate::orm::filter::Filter;
use crate::orm::query::{BuiltQuery, OrmQueryBuilder};

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
    /// Returns an error if any query values cannot be converted to WASI data types.
    pub fn build(self) -> Result<BuiltQuery> {
        let mut statement = Query::delete();
        statement.from_table(Alias::new(M::TABLE));

        for filter in self.filters {
            statement.and_where(filter);
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
            "DeleteBuilder generated SQL"
        );

        Ok(BuiltQuery { sql, params })
    }
}
