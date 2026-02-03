use anyhow::Result;
use wasmtime::component::{Accessor, Resource};

use crate::ConnectionProxy;
use crate::host::generated::wasi::sql::readwrite::{
    Connection, Error, Host, HostWithStore, Row, Statement,
};
use crate::host::{WasiSql, WasiSqlCtxView};

impl HostWithStore for WasiSql {
    async fn query<T>(
        accessor: &Accessor<T, Self>, c: Resource<Connection>, q: Resource<Statement>,
    ) -> wasmtime::Result<Result<Vec<Row>, Resource<Error>>> {
        let connection = get_connection(accessor, &c).map_err(wasmtime::Error::from_anyhow)?;
        let statement = get_statement(accessor, &q).map_err(wasmtime::Error::from_anyhow)?;

        // get statement from resource table
        let (query, params) = (statement.query.clone(), statement.params.clone());

        // execute query
        let result = match connection.query(query, params).await {
            Ok(rows) => Ok(rows),
            Err(err) => Err(accessor.with(|mut store| store.get().table.push(err))?),
        };

        Ok(result)
    }

    async fn exec<T>(
        accessor: &Accessor<T, Self>, c: Resource<Connection>, q: Resource<Statement>,
    ) -> wasmtime::Result<Result<u32, Resource<Error>>> {
        let connection = get_connection(accessor, &c).map_err(wasmtime::Error::from_anyhow)?;
        let statement = get_statement(accessor, &q).map_err(wasmtime::Error::from_anyhow)?;

        // get statement from resource table
        let (query, params) = (statement.query.clone(), statement.params.clone());

        // execute query
        let result = match connection.exec(query, params).await {
            Ok(rows) => Ok(rows),
            Err(err) => Err(accessor.with(|mut store| store.get().table.push(err))?),
        };

        Ok(result)
    }
}

impl Host for WasiSqlCtxView<'_> {}

pub fn get_connection<T>(
    accessor: &Accessor<T, WasiSql>, self_: &Resource<ConnectionProxy>,
) -> Result<ConnectionProxy> {
    accessor.with(|mut store| {
        let connection = store.get().table.get(self_)?;
        Ok::<_, Error>(connection.clone())
    })
}

pub fn get_statement<T>(
    accessor: &Accessor<T, WasiSql>, self_: &Resource<Statement>,
) -> Result<Statement> {
    accessor.with(|mut store| {
        let statement = store.get().table.get(self_)?;
        Ok::<_, Error>(statement.clone())
    })
}
