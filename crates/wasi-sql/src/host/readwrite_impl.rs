use anyhow::Result;
use omnia_core::HostView;
use wasmtime::component::{Accessor, Resource};

use crate::host::generated::wasi::sql::readwrite::{
    Connection, Error, Host, HostWithStore, Row, Statement,
};
use crate::host::resource::{ConnectionProxy, Prepared};
use crate::host::types_impl::trace;
use crate::host::{SqlClient, WasiSql};

impl<T> HostWithStore<T> for WasiSql {
    async fn query(
        accessor: &Accessor<T, Self>, c: Resource<Connection>, q: Resource<Statement>,
    ) -> wasmtime::Result<Result<Vec<Row>, Resource<Error>>> {
        let connection = get_connection(accessor, &c).map_err(wasmtime::Error::from_anyhow)?;
        let statement = get_statement(accessor, &q).map_err(wasmtime::Error::from_anyhow)?;

        let result = match connection.query(statement.sql, statement.params).await {
            Ok(rows) => Ok(rows
                .into_iter()
                .enumerate()
                .map(|(index, row)| Row {
                    index: index.to_string(),
                    fields: row.fields.into_iter().map(Into::into).collect(),
                })
                .collect()),
            Err(err) => Err(trace(accessor, err)?),
        };

        Ok(result)
    }

    // the draft counts in a `u32`, so a larger count saturates
    async fn exec(
        accessor: &Accessor<T, Self>, c: Resource<Connection>, q: Resource<Statement>,
    ) -> wasmtime::Result<Result<u32, Resource<Error>>> {
        let connection = get_connection(accessor, &c).map_err(wasmtime::Error::from_anyhow)?;
        let statement = get_statement(accessor, &q).map_err(wasmtime::Error::from_anyhow)?;

        let result = match connection.execute(statement.sql, statement.params).await {
            Ok(changed) => Ok(u32::try_from(changed).unwrap_or(u32::MAX)),
            Err(err) => Err(trace(accessor, err)?),
        };

        Ok(result)
    }
}

pub fn get_connection<T>(
    accessor: &Accessor<T, WasiSql>, self_: &Resource<ConnectionProxy>,
) -> Result<ConnectionProxy> {
    Ok(omnia_core::get_cloned(accessor, self_)?)
}

pub fn get_statement<T>(
    accessor: &Accessor<T, WasiSql>, self_: &Resource<Prepared>,
) -> Result<Prepared> {
    Ok(omnia_core::get_cloned(accessor, self_)?)
}

impl Host for HostView<'_, dyn SqlClient> {}
