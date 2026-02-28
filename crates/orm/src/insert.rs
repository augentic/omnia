use anyhow::Result;
use sea_query::{Alias, SimpleExpr, Value};

use crate::entity::{Entity, EntityValues, values_to_wasi_datatypes};
use crate::query::{Query, QueryBuilder};

/// Builder for constructing INSERT queries.
pub struct InsertBuilder {
    table: String,
    values: Vec<(&'static str, Value)>,
}

impl InsertBuilder {
    /// Creates a new INSERT query builder for the given table.
    #[must_use]
    pub fn new(table: &str) -> Self {
        Self {
            table: table.to_string(),
            values: Vec::new(),
        }
    }

    /// Creates an INSERT builder pre-populated with all fields from an entity instance.
    #[must_use]
    pub fn from_entity<E: EntityValues>(table: &str, entity: &E) -> Self {
        Self {
            table: table.to_string(),
            values: entity.__to_values(),
        }
    }

    /// Creates an INSERT builder from an entity, inferring the table name from [`Entity::TABLE`].
    #[must_use]
    pub fn from<E: Entity + EntityValues>(entity: &E) -> Self {
        Self {
            table: E::TABLE.to_string(),
            values: entity.__to_values(),
        }
    }

    /// Sets a column value for the insert.
    #[must_use]
    pub fn set<V>(mut self, column: &'static str, value: V) -> Self
    where
        V: Into<Value>,
    {
        self.values.push((column, value.into()));
        self
    }

    /// Build the INSERT query.
    ///
    /// # Errors
    ///
    /// Returns an error if any query values cannot be converted to WASI data types.
    pub fn build(self) -> Result<Query> {
        let mut statement = sea_query::Query::insert();
        statement.into_table(Alias::new(&self.table));

        let columns: Vec<_> = self.values.iter().map(|(column, _)| Alias::new(*column)).collect();
        let row: Vec<SimpleExpr> =
            self.values.into_iter().map(|(_, value)| SimpleExpr::Value(value)).collect();

        statement.columns(columns);
        statement.values_panic(row);

        let (sql, values) = statement.build(QueryBuilder);
        let params = values_to_wasi_datatypes(values)?;

        tracing::debug!(
            table = %self.table,
            sql = %sql,
            param_count = params.len(),
            "InsertBuilder generated SQL"
        );

        Ok(Query { sql, params })
    }
}
