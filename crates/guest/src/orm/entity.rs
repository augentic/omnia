use anyhow::{Result, anyhow, bail};
use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use sea_query::{Value, Values};

use super::join::Join;
use super::{DataType, Row};

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

/// Trait for database entities with metadata for query building.
///
/// Typically implemented via the `entity!` macro rather than manually.
pub trait Entity: Sized {
    /// The database table name for this entity.
    const TABLE: &'static str;

    /// Column names to select when fetching this entity.
    fn projection() -> &'static [&'static str];

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
        Value::String(v) => DataType::Str(v),
        Value::ChronoDate(v) => DataType::Date(v.map(|value| value.to_string())),
        Value::ChronoTime(v) => DataType::Time(v.map(|value| value.to_string())),
        Value::ChronoDateTime(v) => DataType::Timestamp(v.map(|value| value.to_string())),
        Value::ChronoDateTimeUtc(v) => DataType::Timestamp(v.map(|value| value.to_rfc3339())),
        Value::Char(v) => DataType::Str(v.map(|ch| ch.to_string())),
        Value::Bytes(v) => DataType::Binary(v),
        _ => {
            bail!("unsupported values require explicit conversion before building the query")
        }
    };
    Ok(data_type)
}

macro_rules! fetch {
    ($($ty:ty => $variant:ident),* $(,)?) => {$(
        impl FetchValue for $ty {
            fn fetch(row: &Row, col: &str) -> anyhow::Result<Self> {
                match row_field(row, col)? {
                    DataType::$variant(Some(v)) => Ok(v.clone()),
                    _ => bail!(concat!("expected ", stringify!($variant), " data type")),
                }
            }
        }
    )*};
}

fetch! {
    bool    => Boolean,
    i32     => Int32,
    i64     => Int64,
    u32     => Uint32,
    u64     => Uint64,
    f32     => Float,
    f64     => Double,
    String  => Str,
    Vec<u8> => Binary,
}

impl FetchValue for DateTime<Utc> {
    fn fetch(row: &Row, col: &str) -> anyhow::Result<Self> {
        parse_timestamp(row_field(row, col)?)
    }
}

impl FetchValue for NaiveDate {
    fn fetch(row: &Row, col: &str) -> anyhow::Result<Self> {
        parse_date(row_field(row, col)?)
    }
}

impl FetchValue for serde_json::Value {
    fn fetch(row: &Row, col: &str) -> anyhow::Result<Self> {
        parse_json(row_field(row, col)?)
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

fn parse_timestamp(value: &DataType) -> Result<DateTime<Utc>> {
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

fn parse_date(value: &DataType) -> Result<NaiveDate> {
    match value {
        DataType::Date(Some(raw)) => NaiveDate::parse_from_str(raw, "%Y-%m-%d")
            .map_err(|_e| anyhow!("unsupported date: {raw}; expected \"%Y-%m-%d\" format")),
        _ => bail!("expected date data type"),
    }
}

fn parse_json(value: &DataType) -> Result<serde_json::Value> {
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
    fn value_to_wasi_datatype_variants() {
        use sea_query::Value;

        let date = NaiveDate::from_ymd_opt(2024, 1, 15).unwrap();
        let dt_utc: DateTime<Utc> = "2024-01-15T10:30:45Z".parse().unwrap();

        assert!(matches!(
            value_to_wasi_datatype(Value::Int(Some(42))).unwrap(),
            DataType::Int32(Some(42))
        ));
        assert!(matches!(
            value_to_wasi_datatype(Value::BigUnsigned(Some(10_000))).unwrap(),
            DataType::Uint64(Some(10_000))
        ));
        assert!(matches!(
            value_to_wasi_datatype(Value::Float(Some(std::f32::consts::PI))).unwrap(),
            DataType::Float(Some(v)) if (v - std::f32::consts::PI).abs() < 0.01
        ));
        assert!(matches!(
            value_to_wasi_datatype(Value::Char(Some('A'))).unwrap(),
            DataType::Str(Some(s)) if s == "A"
        ));
        assert!(matches!(
            value_to_wasi_datatype(Value::Bytes(Some(vec![1, 2, 3]))).unwrap(),
            DataType::Binary(Some(b)) if b == [1, 2, 3]
        ));
        assert!(matches!(
            value_to_wasi_datatype(Value::ChronoDate(Some(date))).unwrap(),
            DataType::Date(Some(s)) if s == "2024-01-15"
        ));
        assert!(matches!(
            value_to_wasi_datatype(Value::ChronoDateTimeUtc(Some(dt_utc))).unwrap(),
            DataType::Timestamp(Some(s)) if s.contains("2024-01-15") && s.contains("10:30:45")
        ));
        assert!(matches!(
            value_to_wasi_datatype(Value::Bool(None)).unwrap(),
            DataType::Boolean(None)
        ));
    }

    #[test]
    fn fetch_wrong_types() {
        use omnia_wasi_sql::Field;

        fn one_field_row(value: DataType) -> Row {
            Row {
                fields: vec![Field {
                    name: "x".to_string(),
                    value,
                }],
                index: "0".to_string(),
            }
        }

        bool::fetch(&one_field_row(DataType::Int32(Some(1))), "x").unwrap_err();

        i32::fetch(&one_field_row(DataType::Str(Some("not a number".to_string()))), "x")
            .unwrap_err();

        i64::fetch(&one_field_row(DataType::Boolean(Some(true))), "x").unwrap_err();

        String::fetch(&one_field_row(DataType::Int32(Some(42))), "x").unwrap_err();

        <Vec<u8>>::fetch(&one_field_row(DataType::Str(Some("not binary".to_string()))), "x")
            .unwrap_err();

        let err = DateTime::<Utc>::fetch(
            &one_field_row(DataType::Timestamp(Some("invalid".into()))),
            "x",
        )
        .unwrap_err();
        assert!(err.to_string().contains("unsupported timestamp"));

        serde_json::Value::fetch(&one_field_row(DataType::Str(Some("not json".to_string()))), "x")
            .unwrap_err();
    }
}
