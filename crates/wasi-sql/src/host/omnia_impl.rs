use std::sync::Arc;

use omnia_core::HostView;
use wasmtime::component::Accessor;

use crate::host::generated::omnia::sql::database::{Host, HostWithStore};
use crate::host::resource::Connection;
use crate::host::{Error, Result, Row, SqlClient, Statement, Value, WasiSql};

impl<T> HostWithStore<T> for WasiSql {
    async fn query(
        accessor: &Accessor<T, Self>, db: String, sql: String, params: Vec<Value>,
    ) -> Result<Vec<Row>> {
        Ok(open(accessor, db).await?.query(sql, params).await?)
    }

    async fn execute(
        accessor: &Accessor<T, Self>, db: String, sql: String, params: Vec<Value>,
    ) -> Result<u64> {
        Ok(open(accessor, db).await?.execute(sql, params).await?)
    }

    async fn batch(
        accessor: &Accessor<T, Self>, db: String, statements: Vec<Statement>,
    ) -> Result<Vec<u64>> {
        Ok(open(accessor, db).await?.batch(statements).await?)
    }
}

async fn open<T>(
    accessor: &Accessor<T, WasiSql>, db: String,
) -> anyhow::Result<Arc<dyn Connection>> {
    accessor.with(|mut store| store.get().client.open(db)).await
}

impl Host for HostView<'_, dyn SqlClient> {
    fn convert_error(&mut self, err: Error) -> wasmtime::Result<Error> {
        Ok(err)
    }
}
