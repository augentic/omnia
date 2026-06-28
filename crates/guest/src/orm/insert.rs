use std::marker::PhantomData;

use anyhow::Result;
use sea_query::{Alias, OnConflict, SimpleExpr, Value};

use super::entity::{Entity, EntityValues};
use super::query::{Query, finish};

/// Builder for constructing INSERT queries.
pub struct InsertBuilder<M: Entity> {
    values: Vec<(&'static str, Value)>,
    conflict: Option<Conflict>,
    _marker: PhantomData<M>,
}

struct Conflict {
    target: Vec<&'static str>,
    action: ConflictAction,
}

enum ConflictAction {
    Nothing,
    Update(Vec<&'static str>),
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

    /// Handle conflicts on the specified target columns. The default action is `DO NOTHING`;
    /// chain [`Self::do_update`] or [`Self::do_update_all`] to switch to `DO UPDATE`.
    #[must_use]
    pub fn on_conflict_columns(mut self, columns: &[&'static str]) -> Self {
        self.conflict = Some(Conflict {
            target: columns.to_vec(),
            action: ConflictAction::Nothing,
        });
        self
    }

    /// Shorthand for a single-column conflict target.
    #[must_use]
    pub fn on_conflict(self, column: &'static str) -> Self {
        self.on_conflict_columns(&[column])
    }

    /// On conflict, do nothing. (Default action for [`Self::on_conflict_columns`]; provided
    /// for explicit-verb call sites.)
    #[must_use]
    pub fn do_nothing(mut self) -> Self {
        if let Some(conflict) = self.conflict.as_mut() {
            conflict.action = ConflictAction::Nothing;
        }
        self
    }

    /// On conflict, update the specified columns with excluded (new) values.
    #[must_use]
    pub fn do_update(mut self, columns: &[&'static str]) -> Self {
        if let Some(conflict) = self.conflict.as_mut() {
            conflict.action = ConflictAction::Update(columns.to_vec());
        }
        self
    }

    /// On conflict, update all columns except the conflict target.
    #[must_use]
    pub fn do_update_all(mut self) -> Self {
        if let Some(conflict) = self.conflict.as_mut() {
            let update_cols: Vec<&'static str> = self
                .values
                .iter()
                .map(|(col, _)| *col)
                .filter(|col| !conflict.target.contains(col))
                .collect();
            conflict.action = ConflictAction::Update(update_cols);
        }
        self
    }

    /// Build the INSERT query.
    ///
    /// # Errors
    ///
    /// Returns an error if any query values cannot be converted to WASI data types.
    pub fn build(self) -> Result<Query> {
        let mut statement = sea_query::Query::insert();
        statement.into_table(Alias::new(M::TABLE));

        let columns: Vec<_> = self.values.iter().map(|(column, _)| Alias::new(*column)).collect();
        let row: Vec<SimpleExpr> =
            self.values.into_iter().map(|(_, value)| SimpleExpr::Value(value)).collect();

        statement.columns(columns);
        statement.values_panic(row);

        if let Some(Conflict { target, action }) = self.conflict {
            let mut on_conflict = OnConflict::columns(target.into_iter().map(Alias::new));
            match action {
                ConflictAction::Nothing => {
                    on_conflict.do_nothing();
                }
                ConflictAction::Update(cols) => {
                    on_conflict.update_columns(cols.into_iter().map(Alias::new));
                }
            }
            statement.on_conflict(on_conflict);
        }

        finish(&statement, M::TABLE, "insert")
    }
}
