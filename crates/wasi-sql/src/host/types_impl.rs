use anyhow::Result;
use wasmtime::component::{Access, Accessor, Resource};

use crate::host::generated::wasi::sql::types::{
    Connection, DataType as WitDataType, Error, Host, HostConnection, HostConnectionWithStore,
    HostError, HostErrorWithStore, HostStatement, HostStatementWithStore, Statement,
};
use crate::host::resource::ConnectionProxy;
use crate::host::{DataType, WasiSql, WasiSqlView};

impl<T> HostConnectionWithStore<T> for WasiSql {
    async fn open(
        accessor: &Accessor<T, Self>, name: String,
    ) -> wasmtime::Result<Result<Resource<Connection>, Resource<Error>>> {
        let open_conn = accessor.with(|mut store| store.get().client.open(name)).await;

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
        accessor: &Accessor<T, Self>, query: String, params: Vec<WitDataType>,
    ) -> impl std::future::Future<Output = wasmtime::Result<Result<Resource<Statement>, Resource<Error>>>>
    {
        let statement = Statement {
            query,
            params: params.into_iter().map(from_wit_data_type).collect(),
        };
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

impl Host for WasiSqlView<'_> {
    fn convert_error(&mut self, err: Error) -> Result<Error, wasmtime::Error> {
        Ok(err)
    }
}

impl HostConnection for WasiSqlView<'_> {}
impl HostStatement for WasiSqlView<'_> {}
impl HostError for WasiSqlView<'_> {}

fn from_wit_data_type(dt: WitDataType) -> DataType {
    match dt {
        WitDataType::Int32(v) => DataType::Int32(v),
        WitDataType::Int64(v) => DataType::Int64(v),
        WitDataType::Uint32(v) => DataType::Uint32(v),
        WitDataType::Uint64(v) => DataType::Uint64(v),
        WitDataType::Float(v) => DataType::Float(v),
        WitDataType::Double(v) => DataType::Double(v),
        WitDataType::Str(v) => DataType::Str(v),
        WitDataType::Boolean(v) => DataType::Boolean(v),
        WitDataType::Date(v) => DataType::Date(v),
        WitDataType::Time(v) => DataType::Time(v),
        WitDataType::Timestamp(v) => DataType::Timestamp(v),
        WitDataType::Binary(v) => DataType::Binary(v),
    }
}
