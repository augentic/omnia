//! # WASI SQL WIT implementation

// Bindings for the `wasi:sql` world.
// See (<https://github.com/augentic/wasi-sql/>)
mod generated {
    #![expect(missing_docs, clippy::same_length_and_capacity, reason = "wit-bindgen output")]

    wit_bindgen::generate!({
        world: "imports",
        path: "wit",
        generate_all,
    });
}

pub use self::generated::wasi::sql::*;

impl From<omnia_backend::sql::DataType> for types::DataType {
    fn from(value: omnia_backend::sql::DataType) -> Self {
        match value {
            omnia_backend::sql::DataType::Int32(v) => Self::Int32(v),
            omnia_backend::sql::DataType::Int64(v) => Self::Int64(v),
            omnia_backend::sql::DataType::Uint32(v) => Self::Uint32(v),
            omnia_backend::sql::DataType::Uint64(v) => Self::Uint64(v),
            omnia_backend::sql::DataType::Float(v) => Self::Float(v),
            omnia_backend::sql::DataType::Double(v) => Self::Double(v),
            omnia_backend::sql::DataType::Str(v) => Self::Str(v),
            omnia_backend::sql::DataType::Boolean(v) => Self::Boolean(v),
            omnia_backend::sql::DataType::Date(v) => Self::Date(v),
            omnia_backend::sql::DataType::Time(v) => Self::Time(v),
            omnia_backend::sql::DataType::Timestamp(v) => Self::Timestamp(v),
            omnia_backend::sql::DataType::Binary(v) => Self::Binary(v),
        }
    }
}

impl From<types::DataType> for omnia_backend::sql::DataType {
    fn from(value: types::DataType) -> Self {
        match value {
            types::DataType::Int32(v) => Self::Int32(v),
            types::DataType::Int64(v) => Self::Int64(v),
            types::DataType::Uint32(v) => Self::Uint32(v),
            types::DataType::Uint64(v) => Self::Uint64(v),
            types::DataType::Float(v) => Self::Float(v),
            types::DataType::Double(v) => Self::Double(v),
            types::DataType::Str(v) => Self::Str(v),
            types::DataType::Boolean(v) => Self::Boolean(v),
            types::DataType::Date(v) => Self::Date(v),
            types::DataType::Time(v) => Self::Time(v),
            types::DataType::Timestamp(v) => Self::Timestamp(v),
            types::DataType::Binary(v) => Self::Binary(v),
        }
    }
}

impl From<types::Field> for omnia_backend::sql::Field {
    fn from(field: types::Field) -> Self {
        Self {
            name: field.name,
            value: field.value.into(),
        }
    }
}

impl From<types::Row> for omnia_backend::sql::Row {
    fn from(row: types::Row) -> Self {
        Self {
            index: row.index,
            fields: row.fields.into_iter().map(Into::into).collect(),
        }
    }
}
