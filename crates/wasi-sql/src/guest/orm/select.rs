use std::marker::PhantomData;

use anyhow::{Context, Result};
use sea_query::{Alias, ColumnRef, IntoIden, Order, Query, SimpleExpr};

use crate::orm::TableStore;
use crate::orm::entity::{Entity, values_to_wasi_datatypes};
use crate::orm::filter::Filter;
use crate::orm::join::{Join, JoinSpec};
use crate::orm::query::{BuiltQuery, OrmQueryBuilder};

/// Builder for constructing SELECT queries.
pub struct SelectBuilder<M: Entity> {
    filters: Vec<SimpleExpr>,
    limit: Option<u64>,
    offset: Option<u64>,
    order: Vec<(ColumnRef, Order)>,
    joins: Vec<JoinSpec>,
    _marker: PhantomData<M>,
}

impl<M: Entity> Default for SelectBuilder<M> {
    fn default() -> Self {
        let ordering = M::ordering()
            .into_iter()
            .map(|spec| (table_column(spec.table.unwrap_or(M::TABLE), spec.column), spec.order))
            .collect();

        let joins = M::joins().into_iter().map(|join| join.into_join_spec(M::TABLE)).collect();

        Self {
            filters: Vec::new(),
            limit: None,
            offset: None,
            order: ordering,
            joins,
            _marker: PhantomData,
        }
    }
}

impl<M: Entity> SelectBuilder<M> {
    /// Creates a new SELECT query builder.
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

    /// Adds ascending ORDER BY clause.
    #[must_use]
    pub fn order_by(mut self, table: Option<&'static str>, column: &'static str) -> Self {
        let table = table.unwrap_or(M::TABLE);
        self.order.push((table_column(table, column), Order::Asc));
        self
    }

    /// Adds descending ORDER BY clause.
    #[must_use]
    pub fn order_by_desc(mut self, table: Option<&'static str>, column: &'static str) -> Self {
        let table = table.unwrap_or(M::TABLE);
        self.order.push((table_column(table, column), Order::Desc));
        self
    }

    /// Adds a JOIN clause to the query.
    #[must_use]
    pub fn join(mut self, join: Join) -> Self {
        self.joins.push(join.into_join_spec(M::TABLE));
        self
    }

    /// Consumes the builder, executes the query against the provider, and maps rows to the model.
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails to build, execute, or if row conversion to the model fails.
    pub async fn fetch(self, provider: &impl TableStore, pool_name: &str) -> Result<Vec<M>> {
        let BuiltQuery { sql, params } = self.build().context("failed building query")?;

        let rows =
            provider.query(pool_name.to_string(), sql, params).await.context("query failed")?;

        let models = rows
            .iter()
            .map(M::from_row)
            .collect::<Result<Vec<_>>>()
            .context("row conversion failed")?;

        Ok(models)
    }

    /// Build the SELECT query.
    ///
    /// # Errors
    ///
    /// Returns an error if query values cannot be converted to WASI data types.
    pub fn build(self) -> Result<BuiltQuery> {
        let mut statement = Query::select();

        // Build column specs lookup map
        let column_specs = M::column_specs();
        let spec_map: std::collections::HashMap<&str, (&str, &str)> = column_specs
            .into_iter()
            .map(|(field, table, column)| (field, (table, column)))
            .collect();

        // Build columns with proper table qualification
        for field in M::projection() {
            if let Some(&(table, column)) = spec_map.get(field) {
                // Use specified table.column AS field
                statement
                    .expr_as(SimpleExpr::Column(table_column(table, column)), Alias::new(*field));
            } else {
                // Auto-qualify with main table
                statement.column(table_column(M::TABLE, field));
            }
        }

        statement.from(Alias::new(M::TABLE));

        for JoinSpec {
            table,
            alias,
            on,
            kind,
        } in self.joins
        {
            let table_alias = Alias::new(table);
            if let Some(alias) = alias {
                statement.join_as(kind, table_alias, Alias::new(alias), on);
            } else {
                statement.join(kind, table_alias, on);
            }
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
            statement.order_by(column, order);
        }

        let (sql, values) = statement.build(OrmQueryBuilder::default());
        let params = values_to_wasi_datatypes(values)?;

        tracing::debug!(
            table = M::TABLE,
            sql = %sql,
            param_count = params.len(),
            "SelectBuilder generated SQL"
        );

        Ok(BuiltQuery { sql, params })
    }
}

pub fn table_column(table: &str, column: &str) -> ColumnRef {
    ColumnRef::TableColumn(Alias::new(table).into_iden(), Alias::new(column).into_iden())
}
