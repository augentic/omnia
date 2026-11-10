use anyhow::Result;
use omnia_core::HostView;
use wasmtime::component::{Accessor, Resource};

use crate::ConnectionProxy;
use crate::host::generated::wasi::sql::readwrite::{
    Connection, Error, Host, HostWithStore, Row as WitRow, Statement,
};
use crate::host::generated::wasi::sql::types::{DataType as WitDataType, Field as WitField};
use crate::host::{DataType, Field, Row, SqlClient, WasiSql};

pub fn get_connection<T>(
    accessor: &Accessor<T, WasiSql>, self_: &Resource<ConnectionProxy>,
) -> Result<ConnectionProxy> {
    Ok(omnia_core::get_cloned(accessor, self_)?)
}

pub fn get_statement<T>(
    accessor: &Accessor<T, WasiSql>, self_: &Resource<Statement>,
) -> Result<Statement> {
    Ok(omnia_core::get_cloned(accessor, self_)?)
}

fn to_wit_row(row: Row) -> WitRow {
    WitRow {
        index: row.index,
        fields: row.fields.into_iter().map(to_wit_field).collect(),
    }
}

fn to_wit_field(field: Field) -> WitField {
    WitField {
        name: field.name,
        value: to_wit_data_type(field.value),
    }
}

fn to_wit_data_type(dt: DataType) -> WitDataType {
    match dt {
        DataType::Int32(v) => WitDataType::Int32(v),
        DataType::Int64(v) => WitDataType::Int64(v),
        DataType::Uint32(v) => WitDataType::Uint32(v),
        DataType::Uint64(v) => WitDataType::Uint64(v),
        DataType::Float(v) => WitDataType::Float(v),
        DataType::Double(v) => WitDataType::Double(v),
        DataType::Str(v) => WitDataType::Str(v),
        DataType::Boolean(v) => WitDataType::Boolean(v),
        DataType::Date(v) => WitDataType::Date(v),
        DataType::Time(v) => WitDataType::Time(v),
        DataType::Timestamp(v) => WitDataType::Timestamp(v),
        DataType::Binary(v) => WitDataType::Binary(v),
    }
}

impl<T> HostWithStore<T> for WasiSql {
    async fn query(
        accessor: &Accessor<T, Self>, c: Resource<Connection>, q: Resource<Statement>,
    ) -> wasmtime::Result<Result<Vec<WitRow>, Resource<Error>>> {
        let connection = get_connection(accessor, &c).map_err(wasmtime::Error::from_anyhow)?;
        let statement = get_statement(accessor, &q).map_err(wasmtime::Error::from_anyhow)?;

        let (query, params) = (statement.query.clone(), statement.params.clone());

        let result = match connection.query(query, params).await {
            Ok(rows) => Ok(rows.into_iter().map(to_wit_row).collect()),
            Err(err) => Err(accessor.with(|mut store| store.get().table.push(Error::from(err)))?),
        };

        Ok(result)
    }

    async fn exec(
        accessor: &Accessor<T, Self>, c: Resource<Connection>, q: Resource<Statement>,
    ) -> wasmtime::Result<Result<u32, Resource<Error>>> {
        let connection = get_connection(accessor, &c).map_err(wasmtime::Error::from_anyhow)?;
        let statement = get_statement(accessor, &q).map_err(wasmtime::Error::from_anyhow)?;

        let (query, params) = (statement.query.clone(), statement.params.clone());

        let result = match connection.exec(query, params).await {
            Ok(rows) => Ok(rows),
            Err(err) => Err(accessor.with(|mut store| store.get().table.push(Error::from(err)))?),
        };

        Ok(result)
    }
}

impl Host for HostView<'_, dyn SqlClient> {}
