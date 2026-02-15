use std::marker::PhantomData;

use anyhow::Result;
use sea_query::{Alias, OnConflict, Query, SimpleExpr, Value};

use crate::orm::entity::{Entity, EntityValues, values_to_wasi_datatypes};
use crate::orm::query::{BuiltQuery, OrmQueryBuilder};

/// Builder for constructing INSERT queries.
pub struct InsertBuilder<M: Entity> {
    values: Vec<(&'static str, Value)>,
    conflict: Option<ConflictStrategy>,
    _marker: PhantomData<M>,
}

enum ConflictStrategy {
    DoNothing { target: ConflictTarget },
    DoUpdate { target: ConflictTarget, columns: Vec<&'static str> },
}

enum ConflictTarget {
    Columns(Vec<&'static str>),
}

impl<M: Entity> Default for InsertBuilder<M> {
    fn default() -> Self {
        Self {
            values: Vec::new(),
            conflict: None,
            _marker: PhantomData,
        }
    }
}

impl<M: Entity> InsertBuilder<M> {
    /// Creates a new INSERT query builder.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Populate all fields from an entity instance.
    #[must_use]
    pub fn from_entity(entity: &M) -> Self
    where
        M: EntityValues,
    {
        Self {
            values: entity.__to_values(),
            conflict: None,
            _marker: PhantomData,
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

    /// Handle conflicts on specified columns. Call ``do_update()`` or ``do_nothing()`` after.
    #[must_use]
    pub fn on_conflict_columns(mut self, columns: &[&'static str]) -> Self {
        self.conflict = Some(ConflictStrategy::DoNothing {
            target: ConflictTarget::Columns(columns.to_vec()),
        });
        self
    }

    /// Shorthand for single column conflict
    #[must_use]
    pub fn on_conflict(self, column: &'static str) -> Self {
        self.on_conflict_columns(&[column])
    }

    /// On conflict, do nothing (ignore the insert)
    #[must_use]
    pub fn do_nothing(mut self) -> Self {
        if let Some(ConflictStrategy::DoNothing { target }) = self.conflict.take() {
            self.conflict = Some(ConflictStrategy::DoNothing { target });
        }
        self
    }

    /// On conflict, update the specified columns with excluded (new) values
    #[must_use]
    pub fn do_update(mut self, columns: &[&'static str]) -> Self {
        if let Some(conflict) = self.conflict.take() {
            let target = match conflict {
                ConflictStrategy::DoNothing { target }
                | ConflictStrategy::DoUpdate { target, .. } => target,
            };
            self.conflict = Some(ConflictStrategy::DoUpdate {
                target,
                columns: columns.to_vec(),
            });
        }
        self
    }

    /// On conflict, update all columns except the conflict target
    #[must_use]
    pub fn do_update_all(mut self) -> Self {
        if let Some(conflict) = self.conflict.take() {
            let target = match conflict {
                ConflictStrategy::DoNothing { target }
                | ConflictStrategy::DoUpdate { target, .. } => target,
            };
            let conflict_cols: Vec<&str> = match &target {
                ConflictTarget::Columns(cols) => cols.clone(),
            };
            let update_cols: Vec<&'static str> = self
                .values
                .iter()
                .map(|(col, _)| *col)
                .filter(|col| !conflict_cols.contains(col))
                .collect();

            self.conflict = Some(ConflictStrategy::DoUpdate {
                target,
                columns: update_cols,
            });
        }
        self
    }

    /// Build the INSERT query.
    ///
    /// # Errors
    ///
    /// Returns an error if any query values cannot be converted to WASI data types.
    pub fn build(self) -> Result<BuiltQuery> {
        let mut statement = Query::insert();
        statement.into_table(Alias::new(M::TABLE));

        let columns: Vec<_> = self.values.iter().map(|(column, _)| Alias::new(*column)).collect();
        let row: Vec<SimpleExpr> =
            self.values.into_iter().map(|(_, value)| SimpleExpr::Value(value)).collect();

        statement.columns(columns);
        statement.values_panic(row);

        // Handle ON CONFLICT clause
        if let Some(conflict) = self.conflict {
            let on_conflict = match conflict {
                ConflictStrategy::DoNothing { target } => {
                    let ConflictTarget::Columns(cols) = target;
                    OnConflict::columns(cols.into_iter().map(Alias::new)).do_nothing().to_owned()
                }
                ConflictStrategy::DoUpdate { target, columns } => {
                    let ConflictTarget::Columns(cols) = target;
                    OnConflict::columns(cols.into_iter().map(Alias::new))
                        .update_columns(columns.into_iter().map(Alias::new))
                        .to_owned()
                }
            };

            statement.on_conflict(on_conflict);
        }

        let (sql, values) = statement.build(OrmQueryBuilder::default());
        let params = values_to_wasi_datatypes(values)?;

        tracing::debug!(
            table = M::TABLE,
            sql = %sql,
            param_count = params.len(),
            "InsertBuilder generated SQL"
        );

        Ok(BuiltQuery { sql, params })
    }
}
