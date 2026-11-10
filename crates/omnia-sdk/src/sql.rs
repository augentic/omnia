//! SQL table value types and typed row decoding.

use std::error::Error;
use std::fmt;

pub use omnia_guest_macros::FromRow;

/// Common SQL scalar values, mirroring `wasi:sql/types.data-type`.
#[derive(Clone, Debug, PartialEq)]
pub enum DataType {
    /// Signed 32-bit integer.
    Int32(Option<i32>),
    /// Signed 64-bit integer.
    Int64(Option<i64>),
    /// Unsigned 32-bit integer.
    Uint32(Option<u32>),
    /// Unsigned 64-bit integer.
    Uint64(Option<u64>),
    /// 32-bit floating-point number.
    Float(Option<f32>),
    /// 64-bit floating-point number.
    Double(Option<f64>),
    /// UTF-8 string.
    Str(Option<String>),
    /// Boolean value.
    Boolean(Option<bool>),
    /// ISO-8601 date string.
    Date(Option<String>),
    /// ISO-8601 time string.
    Time(Option<String>),
    /// ISO-8601 timestamp string.
    Timestamp(Option<String>),
    /// Opaque binary bytes.
    Binary(Option<Vec<u8>>),
}

impl DataType {
    /// Whether this column value is `NULL`.
    #[must_use]
    pub const fn is_null(&self) -> bool {
        matches!(
            self,
            Self::Int32(None)
                | Self::Int64(None)
                | Self::Uint32(None)
                | Self::Uint64(None)
                | Self::Float(None)
                | Self::Double(None)
                | Self::Str(None)
                | Self::Boolean(None)
                | Self::Date(None)
                | Self::Time(None)
                | Self::Timestamp(None)
                | Self::Binary(None)
        )
    }
}

/// One named column in a result [`Row`].
#[derive(Clone, Debug, PartialEq)]
pub struct Field {
    /// Column name.
    pub name: String,
    /// Column value.
    pub value: DataType,
}

/// One result row returned by a SQL query.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Row {
    /// Row index string.
    pub index: String,
    /// Ordered fields in the row.
    pub fields: Vec<Field>,
}

impl Row {
    /// Read `column` as `T`.
    ///
    /// `T` is any [`FromColumn`]: the integer, floating-point, boolean,
    /// string, and binary scalars, or `Option` of one for a nullable column.
    ///
    /// # Errors
    ///
    /// Returns [`ColumnError`] when `column` is absent from the row, holds
    /// `NULL` for a non-`Option` `T`, or has an incompatible [`DataType`].
    pub fn get<'a, T: FromColumn<'a>>(&'a self, column: &str) -> Result<T, ColumnError> {
        let value = self
            .fields
            .iter()
            .find(|field| field.name == column)
            .map(|field| &field.value)
            .ok_or_else(|| ColumnError::missing(column))?;
        T::from_column(column, value)
    }
}

/// A failure while reading a typed column from a [`Row`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ColumnError {
    description: String,
}

impl ColumnError {
    /// Create a column error from its description.
    pub fn new(description: impl Into<String>) -> Self {
        Self {
            description: description.into(),
        }
    }

    /// Describe why the column could not be read.
    #[must_use]
    pub fn description(&self) -> &str {
        &self.description
    }

    fn missing(column: &str) -> Self {
        Self::new(format!("row lacks column `{column}`"))
    }

    fn null(column: &str) -> Self {
        Self::new(format!("column `{column}` is null"))
    }

    fn type_mismatch(column: &str, expected: &str, actual: &DataType) -> Self {
        Self::new(format!("column `{column}` is not {expected}: {actual:?}"))
    }

    fn overflow(column: &str, target: &str, err: impl fmt::Display) -> Self {
        Self::new(format!("column `{column}` overflows {target}: {err}"))
    }
}

impl fmt::Display for ColumnError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.description)
    }
}

impl Error for ColumnError {}

/// Decode a Rust value from one SQL column's [`DataType`].
///
/// Conversions only widen: a narrower integer reads as a wider one, `Float`
/// reads as `f64`, and every string-shaped variant (`Str`, `Date`, `Time`,
/// `Timestamp`) reads as a string. `Option<T>` reads `NULL` as `None`.
pub trait FromColumn<'a>: Sized {
    /// Decode `Self` from `value`, the content of `column`.
    ///
    /// # Errors
    ///
    /// Returns [`ColumnError`] when `value` is `NULL` for a non-`Option`
    /// `Self` or has an incompatible [`DataType`].
    fn from_column(column: &str, value: &'a DataType) -> Result<Self, ColumnError>;
}

impl<'a, T: FromColumn<'a>> FromColumn<'a> for Option<T> {
    fn from_column(column: &str, value: &'a DataType) -> Result<Self, ColumnError> {
        if value.is_null() { Ok(None) } else { T::from_column(column, value).map(Some) }
    }
}

impl FromColumn<'_> for i64 {
    fn from_column(column: &str, value: &DataType) -> Result<Self, ColumnError> {
        match value {
            DataType::Int64(Some(v)) => Ok(*v),
            DataType::Int32(Some(v)) => Ok(Self::from(*v)),
            DataType::Uint32(Some(v)) => Ok(Self::from(*v)),
            DataType::Uint64(Some(v)) => {
                Self::try_from(*v).map_err(|err| ColumnError::overflow(column, "i64", err))
            }
            v if v.is_null() => Err(ColumnError::null(column)),
            other => Err(ColumnError::type_mismatch(column, "an integer", other)),
        }
    }
}

impl FromColumn<'_> for i32 {
    fn from_column(column: &str, value: &DataType) -> Result<Self, ColumnError> {
        let wide = i64::from_column(column, value)?;
        Self::try_from(wide).map_err(|err| ColumnError::overflow(column, "i32", err))
    }
}

impl FromColumn<'_> for u64 {
    fn from_column(column: &str, value: &DataType) -> Result<Self, ColumnError> {
        match value {
            DataType::Uint64(Some(v)) => Ok(*v),
            DataType::Uint32(Some(v)) => Ok(Self::from(*v)),
            DataType::Int64(Some(v)) => {
                Self::try_from(*v).map_err(|err| ColumnError::overflow(column, "u64", err))
            }
            DataType::Int32(Some(v)) => {
                Self::try_from(*v).map_err(|err| ColumnError::overflow(column, "u64", err))
            }
            v if v.is_null() => Err(ColumnError::null(column)),
            other => Err(ColumnError::type_mismatch(column, "an integer", other)),
        }
    }
}

impl FromColumn<'_> for u32 {
    fn from_column(column: &str, value: &DataType) -> Result<Self, ColumnError> {
        let wide = u64::from_column(column, value)?;
        Self::try_from(wide).map_err(|err| ColumnError::overflow(column, "u32", err))
    }
}

impl FromColumn<'_> for f64 {
    fn from_column(column: &str, value: &DataType) -> Result<Self, ColumnError> {
        match value {
            DataType::Double(Some(v)) => Ok(*v),
            DataType::Float(Some(v)) => Ok(Self::from(*v)),
            v if v.is_null() => Err(ColumnError::null(column)),
            other => Err(ColumnError::type_mismatch(column, "a float", other)),
        }
    }
}

impl FromColumn<'_> for f32 {
    fn from_column(column: &str, value: &DataType) -> Result<Self, ColumnError> {
        match value {
            DataType::Float(Some(v)) => Ok(*v),
            v if v.is_null() => Err(ColumnError::null(column)),
            other => Err(ColumnError::type_mismatch(column, "a 32-bit float", other)),
        }
    }
}

impl FromColumn<'_> for bool {
    fn from_column(column: &str, value: &DataType) -> Result<Self, ColumnError> {
        match value {
            DataType::Boolean(Some(v)) => Ok(*v),
            v if v.is_null() => Err(ColumnError::null(column)),
            other => Err(ColumnError::type_mismatch(column, "a boolean", other)),
        }
    }
}

impl<'a> FromColumn<'a> for &'a str {
    fn from_column(column: &str, value: &'a DataType) -> Result<Self, ColumnError> {
        match value {
            DataType::Str(Some(v))
            | DataType::Date(Some(v))
            | DataType::Time(Some(v))
            | DataType::Timestamp(Some(v)) => Ok(v.as_str()),
            v if v.is_null() => Err(ColumnError::null(column)),
            other => Err(ColumnError::type_mismatch(column, "a string", other)),
        }
    }
}

impl<'a> FromColumn<'a> for String {
    fn from_column(column: &str, value: &'a DataType) -> Result<Self, ColumnError> {
        <&'a str as FromColumn<'a>>::from_column(column, value).map(str::to_owned)
    }
}

impl<'a> FromColumn<'a> for &'a [u8] {
    fn from_column(column: &str, value: &'a DataType) -> Result<Self, ColumnError> {
        match value {
            DataType::Binary(Some(v)) => Ok(v.as_slice()),
            v if v.is_null() => Err(ColumnError::null(column)),
            other => Err(ColumnError::type_mismatch(column, "binary bytes", other)),
        }
    }
}

impl<'a> FromColumn<'a> for Vec<u8> {
    fn from_column(column: &str, value: &'a DataType) -> Result<Self, ColumnError> {
        <&'a [u8] as FromColumn<'a>>::from_column(column, value).map(<[u8]>::to_vec)
    }
}

/// Decode a domain record from a SQL [`Row`].
///
/// Guest code usually derives this through `omnia_sdk::sql::FromRow`; a
/// hand-written impl reads each column through [`Row::get`].
pub trait FromRow: Sized {
    /// Decode `Self` from `row`.
    ///
    /// # Errors
    ///
    /// Returns an error when a required column is missing, `NULL`, or of an
    /// incompatible [`DataType`].
    fn from_row(row: &Row) -> anyhow::Result<Self>;
}
