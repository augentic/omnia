use anyhow::Result;
use wasmtime::component::{Access, Accessor, Resource};

use crate::host::generated::wasi::sql::types::{
    Connection, DataType, Error, Host, HostConnection, HostConnectionWithStore, HostError,
    HostErrorWithStore, HostStatement, HostStatementWithStore, Statement,
};
use crate::host::resource::ConnectionProxy;
use crate::host::{WasiSql, WasiSqlCtxView};

impl<T> HostConnectionWithStore<T> for WasiSql {
    async fn open(
        accessor: &Accessor<T, Self>, name: String,
    ) -> wasmtime::Result<Result<Resource<Connection>, Resource<Error>>> {
        let open_conn = accessor.with(|mut store| store.get().ctx.open(name)).await;

        let result = match open_conn {
            Ok(conn) => {
                let proxy = omnia_core::Proxy(conn);
                Ok(accessor.with(|mut store| store.get().table.push(proxy))?)
            }
            Err(err) => Err(accessor.with(|mut store| store.get().table.push(Error::from(err)))?),
        };

        Ok(result)
    }

    fn drop(
        mut accessor: Access<'_, T, Self>, rep: Resource<ConnectionProxy>,
    ) -> wasmtime::Result<()> {
        accessor.get().table.delete(rep).map(|_| Ok(()))?
    }
}

impl<T> HostStatementWithStore<T> for WasiSql {
    fn prepare(
        accessor: &Accessor<T, Self>, query: String, params: Vec<DataType>,
    ) -> impl std::future::Future<Output = wasmtime::Result<Result<Resource<Statement>, Resource<Error>>>>
    {
        let statement = Statement { query, params };
        std::future::ready(
            accessor
                .with(|mut store| store.get().table.push(statement))
                .map_err(Into::into)
                .map(Ok),
        )
    }

    fn drop(mut accessor: Access<'_, T, Self>, rep: Resource<Statement>) -> wasmtime::Result<()> {
        Ok(accessor.get().table.delete(rep).map(|_| ())?)
    }
}

impl<T> HostErrorWithStore<T> for WasiSql {
    fn trace(mut host: Access<'_, T, Self>, self_: Resource<Error>) -> wasmtime::Result<String> {
        let err = host.get().table.get(&self_)?;
        Ok(err.trace().to_string())
    }

    fn drop(mut accessor: Access<'_, T, Self>, rep: Resource<Error>) -> wasmtime::Result<()> {
        Ok(accessor.get().table.delete(rep).map(|_| ())?)
    }
}

impl Host for WasiSqlCtxView<'_> {
    fn convert_error(&mut self, err: Error) -> Result<Error, wasmtime::Error> {
        Ok(err)
    }
}

impl HostConnection for WasiSqlCtxView<'_> {}
impl HostStatement for WasiSqlCtxView<'_> {}
impl HostError for WasiSqlCtxView<'_> {}
