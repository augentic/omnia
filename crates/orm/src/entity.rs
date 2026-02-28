use anyhow::{Result, anyhow, bail};
use chrono::{DateTime, NaiveDateTime, Utc};
use sea_query::{Value, Values};

use crate::delete::DeleteBuilder;
use crate::insert::InsertBuilder;
use crate::select::SelectBuilder;
use crate::update::UpdateBuilder;
use crate::{DataType, Row};

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
    (
        table = $table:literal,
        $(#[$meta:meta])*
        pub struct $struct_name:ident {
            $(
                $(#[$field_meta:meta])*
                pub $field_name:ident : $field_type:ty
            ),* $(,)?
        }
    ) => {
        #[allow(missing_docs)]
        $(#[$meta])*
        pub struct $struct_name {
            $(
                $(#[$field_meta])*
                pub $field_name : $field_type
            ),*
        }

        impl $crate::Entity for $struct_name {
            const TABLE: &'static str = $table;
            const COLUMNS: &'static [&'static str] = &[$(stringify!($field_name)),*];

            fn from_row(row: &$crate::Row) -> anyhow::Result<Self> {
                Ok(Self {
                    $(
                        $field_name: <$field_type as $crate::FetchValue>::fetch(row, stringify!($field_name))?,
                    )*
                })
            }
        }

        impl $crate::EntityValues for $struct_name {
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

/// Trait for database entities.
///
/// Typically implemented via the `entity!` macro rather than manually.
pub trait Entity: Sized + Send + Sync {
    /// The database table name for this entity.
    const TABLE: &'static str;

    /// The column names for this entity, in field declaration order.
    const COLUMNS: &'static [&'static str];

    /// Construct an entity instance from a database row.
    ///
    /// # Errors
    ///
    /// Returns an error if any required column is missing or cannot be converted to the expected type.
    fn from_row(row: &Row) -> Result<Self>;

    /// Returns a [`SelectBuilder`] pre-configured with this entity's table and columns.
    #[must_use]
    fn select() -> SelectBuilder {
        SelectBuilder::new(Self::TABLE).columns(Self::COLUMNS.iter().copied())
    }

    /// Returns an [`InsertBuilder`] pre-populated with all fields from this entity instance.
    #[must_use]
    fn insert(&self) -> InsertBuilder
    where
        Self: EntityValues,
    {
        InsertBuilder::from(self)
    }

    /// Returns an [`UpdateBuilder`] pre-configured with this entity's table.
    #[must_use]
    fn update() -> UpdateBuilder {
        UpdateBuilder::new(Self::TABLE)
    }

    /// Returns a [`DeleteBuilder`] pre-configured with this entity's table.
    #[must_use]
    fn delete() -> DeleteBuilder {
        DeleteBuilder::new(Self::TABLE)
    }
}

/// Internal trait for extracting entity values. Automatically implemented by the `entity!` macro.
#[doc(hidden)]
pub trait EntityValues {
    fn __to_values(&self) -> Vec<(&'static str, Value)>;
}

/// Converts `sea_query::Values` to WASI `DataType` values.
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
        Value::ChronoDate(v) => DataType::Date(v.map(|value| value.to_string())),
        Value::ChronoTime(v) => DataType::Time(v.map(|value| value.to_string())),
        Value::ChronoDateTime(v) => DataType::Timestamp(v.map(|value| value.to_string())),
        Value::ChronoDateTimeUtc(v) => DataType::Timestamp(v.map(|value| {
            let dt: DateTime<Utc> = *value;
            dt.to_rfc3339()
        })),
        Value::Char(v) => DataType::Str(v.map(|ch| ch.to_string())),
        Value::Bytes(v) => DataType::Binary(v.map(|bytes| *bytes)),
        _ => {
            bail!("unsupported values require explicit conversion before building the query")
        }
    };
    Ok(data_type)
}

macro_rules! impl_fetch_value {
    ($ty:ty, $convert:ident) => {
        impl FetchValue for $ty {
            fn fetch(row: &Row, col: &str) -> anyhow::Result<Self> {
                $convert(row_field(row, col)?)
            }
        }
    };
}

impl_fetch_value!(bool, as_bool);
impl_fetch_value!(i32, as_i32);
impl_fetch_value!(i64, as_i64);
impl_fetch_value!(u32, as_u32);
impl_fetch_value!(u64, as_u64);
impl_fetch_value!(f32, as_f32);
impl_fetch_value!(f64, as_f64);
impl_fetch_value!(String, as_string);
impl_fetch_value!(Vec<u8>, as_binary);
impl_fetch_value!(DateTime<Utc>, as_timestamp);
impl_fetch_value!(serde_json::Value, as_json);

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

            bail!(
                "unsupported timestamp: {raw}; expected RFC3339 or \"%Y-%m-%d %H:%M:%S%.f\" format"
            )
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn value_to_wasi_numeric_types() {
        use sea_query::Value;

        let val_bool = value_to_wasi_datatype(Value::Bool(Some(true))).unwrap();
        assert!(matches!(val_bool, DataType::Boolean(Some(true))));

        let val_int = value_to_wasi_datatype(Value::Int(Some(42))).unwrap();
        assert!(matches!(val_int, DataType::Int32(Some(42))));

        let val_bigint = value_to_wasi_datatype(Value::BigInt(Some(999))).unwrap();
        assert!(matches!(val_bigint, DataType::Int64(Some(999))));

        let val_tiny = value_to_wasi_datatype(Value::TinyInt(Some(10))).unwrap();
        assert!(matches!(val_tiny, DataType::Int32(Some(10))));

        let val_small = value_to_wasi_datatype(Value::SmallInt(Some(1000))).unwrap();
        assert!(matches!(val_small, DataType::Int32(Some(1000))));

        let val_tiny_u = value_to_wasi_datatype(Value::TinyUnsigned(Some(10))).unwrap();
        assert!(matches!(val_tiny_u, DataType::Uint32(Some(10))));

        let val_small_u = value_to_wasi_datatype(Value::SmallUnsigned(Some(500))).unwrap();
        assert!(matches!(val_small_u, DataType::Uint32(Some(500))));

        let val_unsigned = value_to_wasi_datatype(Value::Unsigned(Some(1000))).unwrap();
        assert!(matches!(val_unsigned, DataType::Uint32(Some(1000))));

        let val_big_u = value_to_wasi_datatype(Value::BigUnsigned(Some(10000))).unwrap();
        assert!(matches!(val_big_u, DataType::Uint64(Some(10000))));

        let val_f32 = value_to_wasi_datatype(Value::Float(Some(std::f32::consts::PI))).unwrap();
        assert!(
            matches!(val_f32, DataType::Float(Some(v)) if (v - std::f32::consts::PI).abs() < 0.01)
        );

        let val_f64 = value_to_wasi_datatype(Value::Double(Some(std::f64::consts::E))).unwrap();
        assert!(
            matches!(val_f64, DataType::Double(Some(v)) if (v - std::f64::consts::E).abs() < 0.001)
        );
    }

    #[test]
    fn value_to_wasi_string_types() {
        use sea_query::Value;

        let val_string =
            value_to_wasi_datatype(Value::String(Some(Box::new("test".to_string())))).unwrap();
        if let DataType::Str(Some(s)) = &val_string {
            assert_eq!(s, "test");
        } else {
            panic!("Expected string");
        }

        let val_char = value_to_wasi_datatype(Value::Char(Some('A'))).unwrap();
        if let DataType::Str(Some(s)) = &val_char {
            assert_eq!(s, "A");
        } else {
            panic!("Expected string from char");
        }
    }

    #[test]
    fn value_to_wasi_binary_types() {
        use sea_query::Value;

        let val = value_to_wasi_datatype(Value::Bytes(Some(Box::new(vec![1, 2, 3])))).unwrap();
        if let DataType::Binary(Some(b)) = &val {
            assert_eq!(b, &vec![1, 2, 3]);
        } else {
            panic!("Expected binary");
        }
    }

    #[test]
    fn value_to_wasi_datetime_types() {
        use chrono::{DateTime, NaiveDate, NaiveDateTime, NaiveTime, Utc};
        use sea_query::Value;

        let date = NaiveDate::from_ymd_opt(2024, 1, 15).unwrap();
        let val_date = value_to_wasi_datatype(Value::ChronoDate(Some(Box::new(date)))).unwrap();
        if let DataType::Date(Some(s)) = &val_date {
            assert_eq!(s, "2024-01-15");
        } else {
            panic!("Expected date string");
        }

        let time = NaiveTime::from_hms_opt(10, 30, 45).unwrap();
        let val_time = value_to_wasi_datatype(Value::ChronoTime(Some(Box::new(time)))).unwrap();
        if let DataType::Time(Some(s)) = &val_time {
            assert!(s.starts_with("10:30:45"));
        } else {
            panic!("Expected time string");
        }

        let dt = NaiveDateTime::parse_from_str("2024-01-15 10:30:45", "%Y-%m-%d %H:%M:%S").unwrap();
        let val_dt = value_to_wasi_datatype(Value::ChronoDateTime(Some(Box::new(dt)))).unwrap();
        if let DataType::Timestamp(Some(s)) = &val_dt {
            assert!(s.starts_with("2024-01-15"));
        } else {
            panic!("Expected timestamp string");
        }

        let dt_utc: DateTime<Utc> = "2024-01-15T10:30:45Z".parse().unwrap();
        let val_dt_utc =
            value_to_wasi_datatype(Value::ChronoDateTimeUtc(Some(Box::new(dt_utc)))).unwrap();
        if let DataType::Timestamp(Some(s)) = &val_dt_utc {
            assert!(s.contains("2024-01-15"));
            assert!(s.contains("10:30:45"));
        } else {
            panic!("Expected timestamp string");
        }
    }

    #[test]
    fn value_to_wasi_null_variants() {
        use sea_query::Value;

        let val_bool = value_to_wasi_datatype(Value::Bool(None)).unwrap();
        assert!(matches!(val_bool, DataType::Boolean(None)));

        let val_int = value_to_wasi_datatype(Value::Int(None)).unwrap();
        assert!(matches!(val_int, DataType::Int32(None)));

        let val_bigint = value_to_wasi_datatype(Value::BigInt(None)).unwrap();
        assert!(matches!(val_bigint, DataType::Int64(None)));

        let val_string = value_to_wasi_datatype(Value::String(None)).unwrap();
        assert!(matches!(val_string, DataType::Str(None)));
    }

    #[test]
    fn as_type_conversion_errors() {
        let result = as_bool(&DataType::Int32(Some(1)));
        result.unwrap_err();

        let result = as_i32(&DataType::Str(Some("not a number".to_string())));
        result.unwrap_err();

        let result = as_i64(&DataType::Boolean(Some(true)));
        result.unwrap_err();

        let result = as_string(&DataType::Int32(Some(42)));
        result.unwrap_err();

        let result = as_binary(&DataType::Str(Some("not binary".to_string())));
        result.unwrap_err();

        let result = as_timestamp(&DataType::Timestamp(Some("invalid date".to_string())));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("unsupported timestamp"));

        let result = as_json(&DataType::Str(Some("not json".to_string())));
        result.unwrap_err();
    }
}
