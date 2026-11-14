use anyhow::Result;
use omnia_core::HostView;
use wasmtime::component::{Access, Accessor, Resource};

use crate::host::generated::omnia::sql::database;
use crate::host::generated::wasi::sql::types::{
    Connection, DataType, Error, Field, Host, HostConnection, HostConnectionWithStore, HostError,
    HostErrorWithStore, HostStatement, HostStatementWithStore, Statement,
};
use crate::host::resource::{ConnectionProxy, Prepared, Trace};
use crate::host::{SqlClient, WasiSql};

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
            Err(err) => Err(trace(accessor, err)?),
        };

        Ok(result)
    }

    fn drop(
        mut accessor: Access<'_, T, Self>, rep: Resource<ConnectionProxy>,
    ) -> wasmtime::Result<()> {
        accessor.get().table.delete(rep).map(|_| Ok(()))?
    }
}

// the draft's error resource carries a failure as its message alone
pub fn trace<T>(
    accessor: &Accessor<T, WasiSql>, err: anyhow::Error,
) -> wasmtime::Result<Resource<Trace>> {
    let trace = Trace::from(database::Error::from(err));
    Ok(accessor.with(|mut store| store.get().table.push(trace))?)
}

impl<T> HostStatementWithStore<T> for WasiSql {
    fn prepare(
        accessor: &Accessor<T, Self>, query: String, params: Vec<DataType>,
    ) -> impl std::future::Future<Output = wasmtime::Result<Result<Resource<Statement>, Resource<Error>>>>
    {
        let statement = Prepared {
            sql: query,
            params: params.into_iter().map(Into::into).collect(),
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
        let Trace(message) = host.get().table.get(&self_)?;
        Ok(message.clone())
    }

    fn drop(mut accessor: Access<'_, T, Self>, rep: Resource<Error>) -> wasmtime::Result<()> {
        Ok(accessor.get().table.delete(rep).map(|_| ())?)
    }
}

impl Host for HostView<'_, dyn SqlClient> {
    fn convert_error(&mut self, err: Error) -> Result<Error, wasmtime::Error> {
        Ok(err)
    }
}

impl HostConnection for HostView<'_, dyn SqlClient> {}
impl HostStatement for HostView<'_, dyn SqlClient> {}
impl HostError for HostView<'_, dyn SqlClient> {}

impl From<database::Error> for Trace {
    fn from(err: database::Error) -> Self {
        let (database::Error::NotFound(message)
        | database::Error::Denied(message)
        | database::Error::Invalid(message)
        | database::Error::Unsupported(message)
        | database::Error::Unavailable(message)
        | database::Error::Other(message)) = err;
        Self(message)
    }
}

impl From<DataType> for database::Value {
    fn from(value: DataType) -> Self {
        match value {
            DataType::Int32(v) => Self::Int32(v),
            DataType::Int64(v) => Self::Int64(v),
            DataType::Uint32(v) => Self::Uint32(v),
            DataType::Uint64(v) => Self::Uint64(v),
            DataType::Float(v) => Self::Float(v),
            DataType::Double(v) => Self::Double(v),
            DataType::Str(v) => Self::Str(v),
            DataType::Boolean(v) => Self::Boolean(v),
            DataType::Date(v) => Self::Date(v),
            DataType::Time(v) => Self::Time(v),
            DataType::Timestamp(v) => Self::Timestamp(v),
            DataType::Binary(v) => Self::Binary(v),
        }
    }
}

impl From<database::Value> for DataType {
    fn from(value: database::Value) -> Self {
        match value {
            database::Value::Int32(v) => Self::Int32(v),
            database::Value::Int64(v) => Self::Int64(v),
            database::Value::Uint32(v) => Self::Uint32(v),
            database::Value::Uint64(v) => Self::Uint64(v),
            database::Value::Float(v) => Self::Float(v),
            database::Value::Double(v) => Self::Double(v),
            database::Value::Str(v) => Self::Str(v),
            database::Value::Boolean(v) => Self::Boolean(v),
            database::Value::Date(v) => Self::Date(v),
            database::Value::Time(v) => Self::Time(v),
            database::Value::Timestamp(v) => Self::Timestamp(v),
            database::Value::Binary(v) => Self::Binary(v),
        }
    }
}

impl From<database::Field> for Field {
    fn from(field: database::Field) -> Self {
        Self {
            name: field.name,
            value: field.value.into(),
        }
    }
}
