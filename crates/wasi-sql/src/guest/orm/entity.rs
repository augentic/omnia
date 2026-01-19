use anyhow::{Result, anyhow, bail};
use chrono::{DateTime, NaiveDateTime, Utc};
use sea_query::{Order, Value, Values};

use crate::orm::join::Join;
use crate::types::{DataType, Row};

/// Trait for types that can be extracted from database rows.
///
/// This trait is implemented for all standard Rust types that can be
/// fetched from a database row (`i32`, `String`, `DateTime`, etc.).
pub trait FetchValue: Sized {
    /// Fetch a value from a row by column name.
    ///
    /// # Errors
    ///
    /// Returns an error if the column is missing or the value cannot be converted to the target type.
    fn fetch(row: &Row, col: &str) -> anyhow::Result<Self>;
}

/// Declares an ORM entity with automatic `Entity` trait implementation.
///
/// # Examples
///
/// ```ignore
/// entity! {
///     table = "posts",
///     pub struct Post {
///         pub id: i32,
///         pub title: String,
///     }
/// }
/// ```
#[macro_export]
macro_rules! entity {
    // With joins and columns
    (
        table = $table:literal,
        columns = [$( ($col_table:literal, $col_name:literal, $col_field:literal) ),* $(,)?],
        joins = [$($join:expr),* $(,)?],
        $(#[$meta:meta])*
        pub struct $struct_name:ident {
            $(pub $field_name:ident : $field_type:ty),* $(,)?
        }
    ) => {
        $(#[$meta])*
        pub struct $struct_name {
            $(pub $field_name : $field_type),*
        }

        impl $crate::orm::Entity for $struct_name {
            const TABLE: &'static str = $table;

            fn projection() -> &'static [&'static str] {
                &[ $( stringify!($field_name) ),* ]
            }

            fn joins() -> Vec<Join> {
                vec![$($join),*]
            }

            fn column_specs() -> Vec<(&'static str, &'static str, &'static str)> {
                vec![$( ($col_field, $col_table, $col_name) ),*]
            }

            fn from_row(row: &$crate::types::Row) -> anyhow::Result<Self> {
                Ok(Self {
                    $(
                        $field_name: <$field_type as $crate::orm::FetchValue>::fetch(row, stringify!($field_name))?,
                    )*
                })
            }
        }

        impl $crate::orm::EntityValues for $struct_name {
            fn __to_values(&self) -> Vec<(&'static str, $crate::__private::Value)> {
                vec![
                    $(
                        (stringify!($field_name), self.$field_name.clone().into()),
                    )*
                ]
            }
        }
    };

    // With joins only
    (
        table = $table:literal,
        joins = [$($join:expr),* $(,)?],
        $(#[$meta:meta])*
        pub struct $struct_name:ident {
            $(pub $field_name:ident : $field_type:ty),* $(,)?
        }
    ) => {
        $(#[$meta])*
        pub struct $struct_name {
            $(pub $field_name : $field_type),*
        }

        impl $crate::orm::Entity for $struct_name {
            const TABLE: &'static str = $table;

            fn projection() -> &'static [&'static str] {
                &[ $( stringify!($field_name) ),* ]
            }

            fn joins() -> Vec<Join> {
                vec![$($join),*]
            }

            fn from_row(row: &$crate::types::Row) -> anyhow::Result<Self> {
                Ok(Self {
                    $(
                        $field_name: <$field_type as $crate::orm::FetchValue>::fetch(row, stringify!($field_name))?,
                    )*
                })
            }
        }

        impl $crate::orm::EntityValues for $struct_name {
            fn __to_values(&self) -> Vec<(&'static str, $crate::__private::Value)> {
                vec![
                    $(
                        (stringify!($field_name), self.$field_name.clone().into()),
                    )*
                ]
            }
        }
    };

    // Without joins - this is for a basic entity
    (
        table = $table:literal,
        $(#[$meta:meta])*
        pub struct $struct_name:ident {
            $(pub $field_name:ident : $field_type:ty),* $(,)?
        }
    ) => {
        $(#[$meta])*
        pub struct $struct_name {
            $(pub $field_name : $field_type),*
        }

        impl $crate::orm::Entity for $struct_name {
            const TABLE: &'static str = $table;

            fn projection() -> &'static [&'static str] {
                &[ $( stringify!($field_name) ),* ]
            }

            fn from_row(row: &$crate::types::Row) -> anyhow::Result<Self> {
                Ok(Self {
                    $(
                        $field_name: <$field_type as $crate::orm::FetchValue>::fetch(row, stringify!($field_name))?,
                    )*
                })
            }
        }

        impl $crate::orm::EntityValues for $struct_name {
            fn __to_values(&self) -> Vec<(&'static str, $crate::__private::Value)> {
                vec![
                    $(
                        (stringify!($field_name), self.$field_name.clone().into()),
                    )*
                ]
            }
        }
    };
}

/// Trait for database entities with metadata for query building.
///
/// Typically implemented via the `entity!` macro rather than manually.
pub trait Entity: Sized {
    /// The database table name for this entity.
    const TABLE: &'static str;

    /// Column names to select when fetching this entity.
    fn projection() -> &'static [&'static str];

    /// Default ordering specification for queries.
    #[must_use]
    fn ordering() -> Vec<OrderSpec> {
        Vec::new()
    }

    /// Default joins to include when querying this entity.
    #[must_use]
    fn joins() -> Vec<Join> {
        Vec::new()
    }

    /// Column specifications for fields from joined tables.
    /// Returns tuples of (``struct_field``, ``source_table``, ``source_column``).
    /// Fields not listed here will be auto-qualified with the main table.
    #[must_use]
    fn column_specs() -> Vec<(&'static str, &'static str, &'static str)> {
        Vec::new()
    }

    /// Construct an entity instance from a database row.
    ///
    /// # Errors
    ///
    /// Returns an error if any required column is missing or cannot be converted to the expected type.
    fn from_row(row: &Row) -> Result<Self>;
}

/// Internal trait for extracting entity values. Automatically implemented by the `entity!` macro.
#[doc(hidden)]
pub trait EntityValues {
    fn __to_values(&self) -> Vec<(&'static str, Value)>;
}

#[derive(Clone)]
pub struct OrderSpec {
    pub table: Option<&'static str>,
    pub column: &'static str,
    pub order: Order,
}

// Outbound conversion (internal use only)
pub fn values_to_wasi_datatypes(values: Values) -> Result<Vec<DataType>> {
    values.into_iter().map(value_to_wasi_datatype).collect()
}

fn value_to_wasi_datatype(value: Value) -> Result<DataType> {
    let data_type = match value {
        Value::Bool(v) => DataType::Boolean(v),
        Value::TinyInt(v) => DataType::Int32(v.map(i32::from)),
        Value::SmallInt(v) => DataType::Int32(v.map(i32::from)),
        Value::Int(v) => DataType::Int32(v),
        Value::BigInt(v) => DataType::Int64(v),
        Value::TinyUnsigned(v) => DataType::Uint32(v.map(u32::from)),
        Value::SmallUnsigned(v) => DataType::Uint32(v.map(u32::from)),
        Value::Unsigned(v) => DataType::Uint32(v),
        Value::BigUnsigned(v) => DataType::Uint64(v),
        Value::Float(v) => DataType::Float(v),
        Value::Double(v) => DataType::Double(v),
        Value::String(v) => DataType::Str(v.map(|value| *value)),
        Value::ChronoDate(v) => DataType::Date(v.map(|value| {
            let date = *value;
            date.to_string() // "%Y-%m-%d"
        })),
        Value::ChronoTime(v) => DataType::Time(v.map(|value| {
            let time = *value;
            time.to_string() // "%H:%M:%S%.f"
        })),
        Value::ChronoDateTime(v) => DataType::Timestamp(v.map(|value| {
            let dt = *value;
            dt.to_string() // "%Y-%m-%d %H:%M:%S%.f"
        })),
        Value::ChronoDateTimeUtc(v) => DataType::Timestamp(v.map(|value| {
            let dt: DateTime<Utc> = *value;
            dt.to_rfc3339() // "%Y-%m-%dT%H:%M:%S%.f%:z"
        })),
        Value::Char(v) => DataType::Str(v.map(|ch| ch.to_string())),
        Value::Bytes(v) => DataType::Binary(v.map(|bytes| *bytes)),
        _ => {
            bail!("unsupported values require explicit conversion before building the query")
        }
    };
    Ok(data_type)
}

// Inbound conversion
impl FetchValue for bool {
    fn fetch(row: &Row, col: &str) -> anyhow::Result<Self> {
        as_bool(row_field(row, col)?)
    }
}

impl FetchValue for i32 {
    fn fetch(row: &Row, col: &str) -> anyhow::Result<Self> {
        as_i32(row_field(row, col)?)
    }
}

impl FetchValue for i64 {
    fn fetch(row: &Row, col: &str) -> anyhow::Result<Self> {
        as_i64(row_field(row, col)?)
    }
}

impl FetchValue for u32 {
    fn fetch(row: &Row, col: &str) -> anyhow::Result<Self> {
        as_u32(row_field(row, col)?)
    }
}

impl FetchValue for u64 {
    fn fetch(row: &Row, col: &str) -> anyhow::Result<Self> {
        as_u64(row_field(row, col)?)
    }
}

impl FetchValue for f32 {
    fn fetch(row: &Row, col: &str) -> anyhow::Result<Self> {
        as_f32(row_field(row, col)?)
    }
}

impl FetchValue for f64 {
    fn fetch(row: &Row, col: &str) -> anyhow::Result<Self> {
        as_f64(row_field(row, col)?)
    }
}

impl FetchValue for String {
    fn fetch(row: &Row, col: &str) -> anyhow::Result<Self> {
        as_string(row_field(row, col)?)
    }
}

impl FetchValue for Vec<u8> {
    fn fetch(row: &Row, col: &str) -> anyhow::Result<Self> {
        as_binary(row_field(row, col)?)
    }
}

impl FetchValue for DateTime<Utc> {
    fn fetch(row: &Row, col: &str) -> anyhow::Result<Self> {
        as_timestamp(row_field(row, col)?)
    }
}

impl FetchValue for serde_json::Value {
    fn fetch(row: &Row, col: &str) -> anyhow::Result<Self> {
        as_json(row_field(row, col)?)
    }
}

impl<T: FetchValue> FetchValue for Option<T> {
    fn fetch(row: &Row, col: &str) -> anyhow::Result<Self> {
        match row_field(row, col) {
            Ok(field) if !is_null(field) => Ok(Some(T::fetch(row, col)?)),
            _ => Ok(None),
        }
    }
}

fn row_field<'a>(row: &'a Row, name: &str) -> Result<&'a DataType> {
    row.fields
        .iter()
        .find(|field| field.name == name)
        .map(|field| &field.value)
        .ok_or_else(|| anyhow!("missing column '{name}'"))
}

const fn is_null(value: &DataType) -> bool {
    matches!(
        value,
        DataType::Boolean(None)
            | DataType::Int32(None)
            | DataType::Int64(None)
            | DataType::Uint32(None)
            | DataType::Uint64(None)
            | DataType::Float(None)
            | DataType::Double(None)
            | DataType::Str(None)
            | DataType::Binary(None)
            | DataType::Date(None)
            | DataType::Time(None)
            | DataType::Timestamp(None)
    )
}

fn as_bool(value: &DataType) -> Result<bool> {
    match value {
        DataType::Boolean(Some(v)) => Ok(*v),
        _ => bail!("expected boolean data type"),
    }
}

fn as_i32(value: &DataType) -> Result<i32> {
    match value {
        DataType::Int32(Some(v)) => Ok(*v),
        _ => bail!("expected int32 data type"),
    }
}

fn as_i64(value: &DataType) -> Result<i64> {
    match value {
        DataType::Int64(Some(v)) => Ok(*v),
        _ => bail!("expected int64 data type"),
    }
}

fn as_u32(value: &DataType) -> Result<u32> {
    match value {
        DataType::Uint32(Some(v)) => Ok(*v),
        _ => bail!("expected uint32 data type"),
    }
}

fn as_u64(value: &DataType) -> Result<u64> {
    match value {
        DataType::Uint64(Some(v)) => Ok(*v),
        _ => bail!("expected uint64 data type"),
    }
}

fn as_f32(value: &DataType) -> Result<f32> {
    match value {
        DataType::Float(Some(v)) => Ok(*v),
        _ => bail!("expected float data type"),
    }
}

fn as_f64(value: &DataType) -> Result<f64> {
    match value {
        DataType::Double(Some(v)) => Ok(*v),
        _ => bail!("expected double data type"),
    }
}

fn as_string(value: &DataType) -> Result<String> {
    match value {
        DataType::Str(Some(raw)) => Ok(raw.clone()),
        _ => bail!("expected string data type"),
    }
}

fn as_binary(value: &DataType) -> Result<Vec<u8>> {
    match value {
        DataType::Binary(Some(bytes)) => Ok(bytes.clone()),
        _ => bail!("expected binary data type"),
    }
}

fn as_timestamp(value: &DataType) -> Result<DateTime<Utc>> {
    match value {
        DataType::Timestamp(Some(raw)) => {
            if let Ok(parsed) = DateTime::parse_from_rfc3339(raw) {
                return Ok(parsed.with_timezone(&Utc));
            }

            if let Ok(parsed) = NaiveDateTime::parse_from_str(raw, "%Y-%m-%d %H:%M:%S%.f") {
                return Ok(DateTime::<Utc>::from_naive_utc_and_offset(parsed, Utc));
            }

            bail!("unsupported timestamp: {raw}")
        }
        _ => bail!("expected timestamp data type"),
    }
}

fn as_json(value: &DataType) -> Result<serde_json::Value> {
    match value {
        DataType::Str(Some(raw)) => Ok(serde_json::from_str(raw)?),
        DataType::Binary(Some(bytes)) => Ok(serde_json::from_slice(bytes)?),
        _ => bail!("expected json compatible data type"),
    }
}
