//! Relational database capability, SQL value types, and typed row decoding.

use std::error::Error;
use std::fmt;
use std::future::Future;

use bytes::Bytes;
pub use omnia_guest_macros::FromRow;

use crate::capability::{self, ErrorKind};

/// Run parameterized SQL against the databases a deployment declares.
///
/// A handle runs statements on one database. A provider's own handle uses
/// [`sql::DEFAULT`](DEFAULT), and [`named`](Self::named) selects another. A
/// database the deployment does not declare fails on first use. Typed row
/// decoding is [`SqlDbExt`].
///
/// A statement numbers its parameters `$1`, `$2`, and so on. Each binds the
/// value at that position, in any order and as often as it appears. The
/// [SQL reference](https://github.com/augentic/omnia/blob/main/docs/reference/sql.md)
/// lists the dialect and column types every provider runs.
#[omnia_guest_macros::capability]
pub trait SqlDb: Send + Sync {
    /// The handle [`named`](Self::named) returns.
    type Named: SqlDb<Named = Self::Named>;

    /// Run `sql` with `params` bound and return the rows it selects.
    fn query(
        &self, sql: &str, params: &[Value],
    ) -> impl Future<Output = capability::Result<Vec<Row>>> + Send;

    /// Run `sql` with `params` bound and return the number of rows it changed.
    fn execute(
        &self, sql: &str, params: &[Value],
    ) -> impl Future<Output = capability::Result<u64>> + Send;

    /// Run `statements` in order, in one transaction, and return each one's changed-row count.
    ///
    /// A failing statement rolls back the whole batch and returns its error.
    fn batch(
        &self, statements: &[Statement],
    ) -> impl Future<Output = capability::Result<Vec<u64>>> + Send;

    /// A handle to the database `name` on the same provider.
    fn named(&self, name: &str) -> Self::Named;
}

/// Typed row decoding over any [`SqlDb`].
///
/// Blanket-implemented for every [`SqlDb`]; never implemented by hand.
pub trait SqlDbExt: SqlDb {
    /// Run `sql` with `params` bound and decode each row it selects as `T`.
    ///
    /// A row that does not decode fails with [`ErrorKind::Invalid`].
    fn query_as<T: FromRow>(
        &self, sql: &str, params: &[Value],
    ) -> impl Future<Output = capability::Result<Vec<T>>> + Send {
        async move { self.query(sql, params).await?.iter().map(T::from_row).collect() }
    }
}

impl<P: SqlDb + ?Sized> SqlDbExt for P {}

/// The database a provider's own [`SqlDb`] handle uses.
pub const DEFAULT: &str = "default";

/// A SQL scalar: a statement parameter or a column value.
///
/// Every case holds an `Option`, and `None` is a typed `NULL`. The `From`
/// impls build the case a Rust value maps to, and `Option` of one maps
/// `None` to that case's `NULL`.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
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
    Binary(Option<Bytes>),
}

impl Value {
    /// Whether this value is `NULL`.
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

macro_rules! value_from {
    ($($ty:ty => $case:ident),* $(,)?) => {
        $(
            impl From<$ty> for Value {
                fn from(value: $ty) -> Self {
                    Self::$case(Some(value))
                }
            }

            impl From<Option<$ty>> for Value {
                fn from(value: Option<$ty>) -> Self {
                    Self::$case(value)
                }
            }
        )*
    };
}

value_from! {
    String => Str,
    i32 => Int32,
    i64 => Int64,
    u32 => Uint32,
    u64 => Uint64,
    f32 => Float,
    f64 => Double,
    bool => Boolean,
    Bytes => Binary,
}

impl From<&str> for Value {
    fn from(value: &str) -> Self {
        Self::Str(Some(value.to_owned()))
    }
}

impl From<Option<&str>> for Value {
    fn from(value: Option<&str>) -> Self {
        Self::Str(value.map(str::to_owned))
    }
}

impl From<Vec<u8>> for Value {
    fn from(value: Vec<u8>) -> Self {
        Self::Binary(Some(Bytes::from(value)))
    }
}

impl From<Option<Vec<u8>>> for Value {
    fn from(value: Option<Vec<u8>>) -> Self {
        Self::Binary(value.map(Bytes::from))
    }
}

/// One named column in a result [`Row`].
#[derive(Clone, Debug, PartialEq)]
pub struct Field {
    /// Column name.
    pub name: String,
    /// Column value.
    pub value: Value,
}

/// One result row returned by a SQL query.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Row {
    /// The row's columns, in select order.
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
    /// `NULL` for a non-`Option` `T`, or has an incompatible [`Value`].
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

/// One statement of a [`batch`](SqlDb::batch): its SQL and the values it binds.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Statement {
    /// The SQL text.
    pub sql: String,

    /// The values bound to the statement's placeholders, in order.
    pub params: Vec<Value>,
}

impl Statement {
    /// A statement running `sql` with `params` bound.
    pub fn new(sql: impl Into<String>, params: impl Into<Vec<Value>>) -> Self {
        Self {
            sql: sql.into(),
            params: params.into(),
        }
    }
}

/// A failure while reading a typed column from a [`Row`].
///
/// It converts into a [`capability::Error`] of kind [`ErrorKind::Invalid`].
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

    fn type_mismatch(column: &str, expected: &str, actual: &Value) -> Self {
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

impl From<ColumnError> for capability::Error {
    fn from(err: ColumnError) -> Self {
        Self::new(ErrorKind::Invalid, err.description)
    }
}

/// Decode a Rust value from one SQL column's [`Value`].
///
/// - Every integer case reads as any integer type its value fits.
/// - `Float` and `Double` read as `f64`, and only `Float` reads as `f32`.
/// - `Boolean` reads as `bool`, and so does an integer `0` or `1`, which is
///   how `SQLite` returns a `BOOLEAN` column.
/// - Every string-shaped case (`Str`, `Date`, `Time`, `Timestamp`) reads as
///   a string.
/// - `Option<T>` reads `NULL` as `None`.
pub trait FromColumn<'a>: Sized {
    /// Decode `Self` from `value`, the content of `column`.
    ///
    /// # Errors
    ///
    /// Returns [`ColumnError`] when `value` is `NULL` for a non-`Option`
    /// `Self` or has an incompatible [`Value`].
    fn from_column(column: &str, value: &'a Value) -> Result<Self, ColumnError>;
}

impl<'a, T: FromColumn<'a>> FromColumn<'a> for Option<T> {
    fn from_column(column: &str, value: &'a Value) -> Result<Self, ColumnError> {
        if value.is_null() { Ok(None) } else { T::from_column(column, value).map(Some) }
    }
}

impl FromColumn<'_> for i64 {
    fn from_column(column: &str, value: &Value) -> Result<Self, ColumnError> {
        match value {
            Value::Int64(Some(v)) => Ok(*v),
            Value::Int32(Some(v)) => Ok(Self::from(*v)),
            Value::Uint32(Some(v)) => Ok(Self::from(*v)),
            Value::Uint64(Some(v)) => {
                Self::try_from(*v).map_err(|err| ColumnError::overflow(column, "i64", err))
            }
            v if v.is_null() => Err(ColumnError::null(column)),
            other => Err(ColumnError::type_mismatch(column, "an integer", other)),
        }
    }
}

impl FromColumn<'_> for i32 {
    fn from_column(column: &str, value: &Value) -> Result<Self, ColumnError> {
        let wide = i64::from_column(column, value)?;
        Self::try_from(wide).map_err(|err| ColumnError::overflow(column, "i32", err))
    }
}

impl FromColumn<'_> for u64 {
    fn from_column(column: &str, value: &Value) -> Result<Self, ColumnError> {
        match value {
            Value::Uint64(Some(v)) => Ok(*v),
            Value::Uint32(Some(v)) => Ok(Self::from(*v)),
            Value::Int64(Some(v)) => {
                Self::try_from(*v).map_err(|err| ColumnError::overflow(column, "u64", err))
            }
            Value::Int32(Some(v)) => {
                Self::try_from(*v).map_err(|err| ColumnError::overflow(column, "u64", err))
            }
            v if v.is_null() => Err(ColumnError::null(column)),
            other => Err(ColumnError::type_mismatch(column, "an integer", other)),
        }
    }
}

impl FromColumn<'_> for u32 {
    fn from_column(column: &str, value: &Value) -> Result<Self, ColumnError> {
        let wide = u64::from_column(column, value)?;
        Self::try_from(wide).map_err(|err| ColumnError::overflow(column, "u32", err))
    }
}

impl FromColumn<'_> for f64 {
    fn from_column(column: &str, value: &Value) -> Result<Self, ColumnError> {
        match value {
            Value::Double(Some(v)) => Ok(*v),
            Value::Float(Some(v)) => Ok(Self::from(*v)),
            v if v.is_null() => Err(ColumnError::null(column)),
            other => Err(ColumnError::type_mismatch(column, "a float", other)),
        }
    }
}

impl FromColumn<'_> for f32 {
    fn from_column(column: &str, value: &Value) -> Result<Self, ColumnError> {
        match value {
            Value::Float(Some(v)) => Ok(*v),
            v if v.is_null() => Err(ColumnError::null(column)),
            other => Err(ColumnError::type_mismatch(column, "a 32-bit float", other)),
        }
    }
}

impl FromColumn<'_> for bool {
    fn from_column(column: &str, value: &Value) -> Result<Self, ColumnError> {
        match value {
            Value::Boolean(Some(v)) => Ok(*v),
            Value::Int32(Some(0))
            | Value::Int64(Some(0))
            | Value::Uint32(Some(0))
            | Value::Uint64(Some(0)) => Ok(false),
            Value::Int32(Some(1))
            | Value::Int64(Some(1))
            | Value::Uint32(Some(1))
            | Value::Uint64(Some(1)) => Ok(true),
            v if v.is_null() => Err(ColumnError::null(column)),
            other => Err(ColumnError::type_mismatch(column, "a boolean", other)),
        }
    }
}

impl<'a> FromColumn<'a> for &'a str {
    fn from_column(column: &str, value: &'a Value) -> Result<Self, ColumnError> {
        match value {
            Value::Str(Some(v))
            | Value::Date(Some(v))
            | Value::Time(Some(v))
            | Value::Timestamp(Some(v)) => Ok(v.as_str()),
            v if v.is_null() => Err(ColumnError::null(column)),
            other => Err(ColumnError::type_mismatch(column, "a string", other)),
        }
    }
}

impl<'a> FromColumn<'a> for String {
    fn from_column(column: &str, value: &'a Value) -> Result<Self, ColumnError> {
        <&'a str as FromColumn<'a>>::from_column(column, value).map(str::to_owned)
    }
}

impl<'a> FromColumn<'a> for &'a [u8] {
    fn from_column(column: &str, value: &'a Value) -> Result<Self, ColumnError> {
        match value {
            Value::Binary(Some(v)) => Ok(v.as_ref()),
            v if v.is_null() => Err(ColumnError::null(column)),
            other => Err(ColumnError::type_mismatch(column, "binary bytes", other)),
        }
    }
}

impl<'a> FromColumn<'a> for Vec<u8> {
    fn from_column(column: &str, value: &'a Value) -> Result<Self, ColumnError> {
        <&'a [u8] as FromColumn<'a>>::from_column(column, value).map(<[u8]>::to_vec)
    }
}

impl FromColumn<'_> for Bytes {
    fn from_column(column: &str, value: &Value) -> Result<Self, ColumnError> {
        match value {
            Value::Binary(Some(v)) => Ok(v.clone()),
            v if v.is_null() => Err(ColumnError::null(column)),
            other => Err(ColumnError::type_mismatch(column, "binary bytes", other)),
        }
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
    /// [`ErrorKind::Invalid`] when a required column is missing, `NULL`, or
    /// of an incompatible [`Value`].
    fn from_row(row: &Row) -> capability::Result<Self>;
}
