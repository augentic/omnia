use std::marker::PhantomData;

use anyhow::Result;
use sea_query::{Alias, OnConflict, SimpleExpr, Value};

use super::entity::{Entity, EntityValues};
use super::query::{Query, finish};

/// Marker: no `ON CONFLICT` target has been set.
pub struct NoConflict;

/// Marker: an `ON CONFLICT` target has been set, enabling the conflict actions.
pub struct ConflictSet;

/// Builder for constructing INSERT queries.
///
/// The `C` type-state gates the conflict actions ([`Self::do_update`],
/// [`Self::do_nothing`], [`Self::do_update_all`]) behind a prior
/// [`Self::on_conflict`] / [`Self::on_conflict_columns`], so an action can never
/// silently no-op against an unset target.
pub struct InsertBuilder<M: Entity, C = NoConflict> {
    values: Vec<(&'static str, Value)>,
    conflict: Option<Conflict>,
    _marker: PhantomData<(M, C)>,
}

struct Conflict {
    target: Vec<&'static str>,
    action: ConflictAction,
}

enum ConflictAction {
    Nothing,
    Update(Vec<&'static str>),
}

impl<M: Entity> Default for InsertBuilder<M, NoConflict> {
    fn default() -> Self {
        Self {
            values: Vec::new(),
            conflict: None,
            _marker: PhantomData,
        }
    }
}

impl<M: Entity> InsertBuilder<M, NoConflict> {
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

    /// Handle conflicts on the specified target columns. The default action is `DO NOTHING`;
    /// chain [`Self::do_update`] or [`Self::do_update_all`] to switch to `DO UPDATE`.
    #[must_use]
    pub fn on_conflict_columns(self, columns: &[&'static str]) -> InsertBuilder<M, ConflictSet> {
        InsertBuilder {
            values: self.values,
            conflict: Some(Conflict {
                target: columns.to_vec(),
                action: ConflictAction::Nothing,
            }),
            _marker: PhantomData,
        }
    }

    /// Shorthand for a single-column conflict target.
    #[must_use]
    pub fn on_conflict(self, column: &'static str) -> InsertBuilder<M, ConflictSet> {
        self.on_conflict_columns(&[column])
    }
}

impl<M: Entity, C> InsertBuilder<M, C> {
    /// Sets a column value for the insert.
    #[must_use]
    pub fn set<V>(mut self, column: &'static str, value: V) -> Self
    where
        V: Into<Value>,
    {
        self.values.push((column, value.into()));
        self
    }
}

impl<M: Entity> InsertBuilder<M, ConflictSet> {
    /// On conflict, do nothing (the default action once a target is set).
    #[must_use]
    pub fn do_nothing(mut self) -> Self {
        self.conflict_mut().action = ConflictAction::Nothing;
        self
    }

    /// On conflict, update the specified columns with excluded (new) values.
    #[must_use]
    pub fn do_update(mut self, columns: &[&'static str]) -> Self {
        self.conflict_mut().action = ConflictAction::Update(columns.to_vec());
        self
    }

    /// On conflict, update all columns except the conflict target.
    #[must_use]
    pub fn do_update_all(mut self) -> Self {
        let target = self.conflict_mut().target.clone();
        let update_cols: Vec<&'static str> =
            self.values.iter().map(|(col, _)| *col).filter(|col| !target.contains(col)).collect();
        self.conflict_mut().action = ConflictAction::Update(update_cols);
        self
    }

    // The `ConflictSet` type-state guarantees a target was set.
    const fn conflict_mut(&mut self) -> &mut Conflict {
        self.conflict.as_mut().expect("ConflictSet guarantees a conflict target")
    }
}

impl<M: Entity, C> InsertBuilder<M, C> {
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
