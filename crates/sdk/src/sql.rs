//! `omnia:sql`, parameterized SQL against the databases a deployment declares.
//!
//! A guest opens a [`Database`] with [`open`], or the [`DEFAULT`] database
//! with [`default`]. Beside the generated methods, a database and
//! a [`Transaction`] bind [`Value`] parameters, which [`params!`] lists:
//!
//! - [`query_as`](Database::query_as) decodes each selected row as a
//!   [`FromRow`]: a derived record, a tuple, or a [`Row`].
//! - [`query_opt`](Database::query_opt) decodes the first selected row.
//! - [`exec`](Database::exec) returns the number of rows a statement changed.
//!
//! The [`Executor`] trait carries the same three, so a fn over
//! `&impl Executor` runs on either. [`ResultSet::read`] collects the rows of
//! a raw query.
//!
//! A statement numbers its parameters `$1`, `$2`, and so on. Each binds the
//! value at that position, in any order and as often as it appears. The
//! [SQL reference](https://github.com/augentic/omnia/blob/main/docs/reference/sql.md)
//! lists the dialect and column types every backend runs.
//!
//! Each call on a database commits before it returns, and a statement that
//! starts or ends a transaction or a savepoint, such as `BEGIN`, fails with
//! the code `invalid`. [`Database::transaction`] runs a closure in a
//! transaction instead, and [`Database::begin`] starts one by hand:
//!
//! - Its statements commit or roll back together, and dropping it rolls back.
//! - After a statement in it fails, the transaction is over and its changes
//!   are rolled back, so `commit` fails with the code `invalid`.
//! - A call on the database made before the transaction ends may wait for
//!   it, so a guest ends the transaction before it awaits one.

use std::fmt;
use std::future::Future;

use bytes::Bytes;
pub use omnia_guest_macros::FromRow;

use crate::Result;
pub use crate::bindings::imports::omnia::sql::*;
pub use crate::sql::database::{Database, Statement};
pub use crate::sql::transactions::Transaction;
pub use crate::sql::types::ResultSet;

/// The database a guest opens when the deployment names no other.
pub const DEFAULT: &str = "default";

/// Open the database `name`, a binding the deployment declares.
///
/// # Errors
///
/// The host's failure, under its code: `not-found` for a database the
/// deployment does not declare.
pub async fn open(name: &str) -> Result<Database> {
    Ok(store::open(name.to_owned()).await?)
}

/// Open the [`DEFAULT`] database.
///
/// # Errors
///
/// As [`open`].
pub async fn default() -> Result<Database> {
    open(DEFAULT).await
}

impl Database {
    /// Start a [`Transaction`] on this database.
    ///
    /// # Errors
    ///
    /// The host's failure, under its code.
    pub async fn begin(&self) -> Result<Transaction> {
        Ok(transactions::begin(self).await?)
    }

    /// Run `work` in a [`Transaction`], committing it when `work` returns
    /// `Ok` and rolling it back when `work` returns `Err`.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use omnia_sdk::sql::{self, Database};
    ///
    /// # async fn transfer(db: &Database) -> omnia_sdk::Result<()> {
    /// let debit = "UPDATE accounts SET balance = balance - $2 WHERE id = $1";
    /// let credit = "UPDATE accounts SET balance = balance + $2 WHERE id = $1";
    /// db.transaction(async |tx| {
    ///     tx.exec(debit, sql::params!["alice", 10_i64]).await?;
    ///     tx.exec(credit, sql::params!["bob", 10_i64]).await?;
    ///     Ok(())
    /// })
    /// .await
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// - `work`'s error, once the transaction is rolled back.
    /// - The host's failure to begin or commit, under its code.
    pub async fn transaction<T>(
        &self, work: impl AsyncFnOnce(&Transaction) -> Result<T>,
    ) -> Result<T> {
        let tx = self.begin().await?;
        match work(&tx).await {
            Ok(value) => {
                tx.commit().await?;
                Ok(value)
            }
            Err(err) => {
                let _ = tx.rollback().await;
                Err(err)
            }
        }
    }

    /// Run `sql` with `params` bound and decode each row it selects as `T`.
    ///
    /// # Errors
    ///
    /// - [`ServerError`](crate::ErrorKind::ServerError) with the code `invalid`
    ///   when a row does not decode as `T`.
    /// - The host's failure, under its code.
    pub async fn query_as<T: FromRow>(&self, sql: &str, params: &[Value]) -> Result<Vec<T>> {
        let selected = self.query(sql.to_owned(), bind(params)).await?;
        decode(&selected.read().await?)
    }

    /// Run `sql` with `params` bound and decode the first row it selects as
    /// `T`, or `None` when it selects none.
    ///
    /// # Errors
    ///
    /// - [`ServerError`](crate::ErrorKind::ServerError) with the code `invalid`
    ///   when the row does not decode as `T`.
    /// - The host's failure, under its code.
    pub async fn query_opt<T: FromRow>(&self, sql: &str, params: &[Value]) -> Result<Option<T>> {
        let selected = self.query(sql.to_owned(), bind(params)).await?;
        first(&selected.read().await?)
    }

    /// Run `sql` with `params` bound and return the number of rows it changed.
    ///
    /// # Errors
    ///
    /// The host's failure, under its code.
    pub async fn exec(&self, sql: &str, params: &[Value]) -> Result<u64> {
        Ok(self.execute(sql.to_owned(), bind(params)).await?)
    }
}

impl Transaction {
    /// Run `sql` with `params` bound and decode each row it selects as `T`.
    ///
    /// A row that does not decode leaves the transaction open.
    ///
    /// # Errors
    ///
    /// - [`ServerError`](crate::ErrorKind::ServerError) with the code `invalid`
    ///   when a row does not decode as `T`.
    /// - The host's failure, under its code.
    pub async fn query_as<T: FromRow>(&self, sql: &str, params: &[Value]) -> Result<Vec<T>> {
        let selected = self.query(sql.to_owned(), bind(params)).await?;
        decode(&selected.read().await?)
    }

    /// Run `sql` with `params` bound and decode the first row it selects as
    /// `T`, or `None` when it selects none.
    ///
    /// A row that does not decode leaves the transaction open.
    ///
    /// # Errors
    ///
    /// - [`ServerError`](crate::ErrorKind::ServerError) with the code `invalid`
    ///   when the row does not decode as `T`.
    /// - The host's failure, under its code.
    pub async fn query_opt<T: FromRow>(&self, sql: &str, params: &[Value]) -> Result<Option<T>> {
        let selected = self.query(sql.to_owned(), bind(params)).await?;
        first(&selected.read().await?)
    }

    /// Run `sql` with `params` bound and return the number of rows it changed.
    ///
    /// # Errors
    ///
    /// The host's failure, under its code.
    pub async fn exec(&self, sql: &str, params: &[Value]) -> Result<u64> {
        Ok(self.execute(sql.to_owned(), bind(params)).await?)
    }
}

/// What runs a statement: a [`Database`] or a [`Transaction`].
///
/// A fn over `&impl Executor` runs its statements wherever its caller does,
/// on the database or inside a transaction. Both types carry these methods
/// inherently too, so calling one needs no import of this trait.
///
/// An executor is `Sync`, so an `async fn` over `&impl Executor` returns a
/// `Send` future, as an axum handler needs.
///
/// # Examples
///
/// ```rust,no_run
/// use omnia_sdk::sql::{self, Executor};
///
/// async fn count(db: &impl Executor, table: &str) -> omnia_sdk::Result<i64> {
///     let sql = format!("SELECT COUNT(*) FROM {table}");
///     let counted: Option<(i64,)> = db.query_opt(&sql, sql::params![]).await?;
///     Ok(counted.map_or(0, |(count,)| count))
/// }
/// ```
pub trait Executor: Sync {
    /// Run `sql` with `params` bound and decode each row it selects as `T`,
    /// as [`Database::query_as`] does.
    fn query_as<T: FromRow>(
        &self, sql: &str, params: &[Value],
    ) -> impl Future<Output = Result<Vec<T>>> + Send;

    /// Run `sql` with `params` bound and decode the first row it selects as
    /// `T`, as [`Database::query_opt`] does.
    fn query_opt<T: FromRow>(
        &self, sql: &str, params: &[Value],
    ) -> impl Future<Output = Result<Option<T>>> + Send;

    /// Run `sql` with `params` bound and return the number of rows it
    /// changed, as [`Database::exec`] does.
    fn exec(&self, sql: &str, params: &[Value]) -> impl Future<Output = Result<u64>> + Send;
}

impl Executor for Database {
    fn query_as<T: FromRow>(
        &self, sql: &str, params: &[Value],
    ) -> impl Future<Output = Result<Vec<T>>> + Send {
        Self::query_as(self, sql, params)
    }

    fn query_opt<T: FromRow>(
        &self, sql: &str, params: &[Value],
    ) -> impl Future<Output = Result<Option<T>>> + Send {
        Self::query_opt(self, sql, params)
    }

    fn exec(&self, sql: &str, params: &[Value]) -> impl Future<Output = Result<u64>> + Send {
        Self::exec(self, sql, params)
    }
}

impl Executor for Transaction {
    fn query_as<T: FromRow>(
        &self, sql: &str, params: &[Value],
    ) -> impl Future<Output = Result<Vec<T>>> + Send {
        Self::query_as(self, sql, params)
    }

    fn query_opt<T: FromRow>(
        &self, sql: &str, params: &[Value],
    ) -> impl Future<Output = Result<Option<T>>> + Send {
        Self::query_opt(self, sql, params)
    }

    fn exec(&self, sql: &str, params: &[Value]) -> impl Future<Output = Result<u64>> + Send {
        Self::exec(self, sql, params)
    }
}

impl ResultSet {
    /// Collect every row the query selected, each column under its name.
    ///
    /// # Errors
    ///
    /// The host's failure, under its code, when the query fails after its
    /// columns arrive.
    pub async fn read(self) -> Result<Vec<Row>> {
        let Self {
            columns,
            rows,
            finished,
        } = self;
        let rows = rows.collect().await;
        finished.await?;

        Ok(rows
            .into_iter()
            .map(|values| Row {
                fields: columns
                    .iter()
                    .zip(values)
                    .map(|(name, value)| Field {
                        name: name.clone(),
                        value: Value::from(value),
                    })
                    .collect(),
            })
            .collect())
    }
}

impl Statement {
    /// A statement of a [`batch`](Database::batch) running `sql` with
    /// `params` bound.
    pub fn new(sql: impl Into<String>, params: &[Value]) -> Self {
        Self {
            sql: sql.into(),
            params: bind(params),
        }
    }
}

fn bind(params: &[Value]) -> Vec<Option<types::Value>> {
    params.iter().cloned().map(Option::from).collect()
}

fn decode<T: FromRow>(rows: &[Row]) -> Result<Vec<T>> {
    rows.iter().map(T::from_row).collect()
}

fn first<T: FromRow>(rows: &[Row]) -> Result<Option<T>> {
    rows.first().map(T::from_row).transpose()
}

/// A SQL scalar: a statement parameter or a column value.
///
/// Each case but [`Null`](Self::Null) is the `omnia:sql` value of its name.
/// The `From` impls build the case a Rust value maps to, and `Option` of one
/// maps `None` to [`Null`](Self::Null). A `u32` binds as an
/// [`Int64`](Self::Int64), and a `u64` binds only once converted to an `i64`.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    /// `NULL`, in a column of any type.
    Null,
    /// For a `BOOLEAN` column.
    Boolean(bool),
    /// For an `INTEGER` column.
    Int32(i32),
    /// For a `BIGINT` column.
    Int64(i64),
    /// For a `REAL` column.
    Float32(f32),
    /// For a `DOUBLE PRECISION` column.
    Float64(f64),
    /// For a `TEXT` column.
    Text(String),
    /// For a `BYTEA` column.
    Binary(Bytes),
    /// For a `DATE` column, as `YYYY-MM-DD`.
    Date(String),
    /// For a `TIME` column, as `HH:MM:SS` with optional fractional seconds.
    Time(String),
    /// For a `TIMESTAMP` column, as `YYYY-MM-DD HH:MM:SS` with optional
    /// fractional seconds.
    Timestamp(String),
    /// For a `TIMESTAMPTZ` column, as RFC 3339. A portable one has a
    /// `+00:00` offset.
    TimestampTz(String),
}

impl Value {
    /// Whether this value is `NULL`.
    #[must_use]
    pub const fn is_null(&self) -> bool {
        matches!(self, Self::Null)
    }
}

#[doc(hidden)]
#[macro_export]
macro_rules! __sql_params {
    ($($value:expr),* $(,)?) => {
        &[$($crate::sql::Value::from($value)),*] as &[$crate::sql::Value]
    };
}

/// List a statement's parameters as `&[Value]`, each through [`Value::from`].
///
/// # Examples
///
/// ```rust
/// use omnia_sdk::sql::{self, Value};
///
/// let params = sql::params![7_i64, "Ōtautahi", None::<String>];
/// assert_eq!(params, [Value::Int64(7), Value::from("Ōtautahi"), Value::Null]);
/// ```
#[doc(inline)]
pub use crate::__sql_params as params;

macro_rules! value_from {
    ($($ty:ty => $case:ident),* $(,)?) => {
        $(
            impl From<$ty> for Value {
                fn from(value: $ty) -> Self {
                    Self::$case(value)
                }
            }
        )*
    };
}

value_from! {
    String => Text,
    i32 => Int32,
    i64 => Int64,
    f32 => Float32,
    f64 => Float64,
    bool => Boolean,
    Bytes => Binary,
}

impl From<u32> for Value {
    fn from(value: u32) -> Self {
        Self::Int64(i64::from(value))
    }
}

impl From<&str> for Value {
    fn from(value: &str) -> Self {
        Self::Text(value.to_owned())
    }
}

impl From<Vec<u8>> for Value {
    fn from(value: Vec<u8>) -> Self {
        Self::Binary(Bytes::from(value))
    }
}

impl<T: Into<Self>> From<Option<T>> for Value {
    fn from(value: Option<T>) -> Self {
        value.map_or(Self::Null, Into::into)
    }
}

impl From<types::Value> for Value {
    fn from(value: types::Value) -> Self {
        match value {
            types::Value::Boolean(v) => Self::Boolean(v),
            types::Value::Int32(v) => Self::Int32(v),
            types::Value::Int64(v) => Self::Int64(v),
            types::Value::Float32(v) => Self::Float32(v),
            types::Value::Float64(v) => Self::Float64(v),
            types::Value::Text(v) => Self::Text(v),
            types::Value::Binary(v) => Self::Binary(Bytes::from(v)),
            types::Value::Date(v) => Self::Date(v),
            types::Value::Time(v) => Self::Time(v),
            types::Value::Timestamp(v) => Self::Timestamp(v),
            types::Value::TimestampTz(v) => Self::TimestampTz(v),
        }
    }
}

/// The wire value a [`Value`] binds as, `None` for [`Null`](Value::Null).
impl From<Value> for Option<types::Value> {
    fn from(value: Value) -> Self {
        let value = match value {
            Value::Null => return None,
            Value::Boolean(v) => types::Value::Boolean(v),
            Value::Int32(v) => types::Value::Int32(v),
            Value::Int64(v) => types::Value::Int64(v),
            Value::Float32(v) => types::Value::Float32(v),
            Value::Float64(v) => types::Value::Float64(v),
            Value::Text(v) => types::Value::Text(v),
            Value::Binary(v) => types::Value::Binary(v.to_vec()),
            Value::Date(v) => types::Value::Date(v),
            Value::Time(v) => types::Value::Time(v),
            Value::Timestamp(v) => types::Value::Timestamp(v),
            Value::TimestampTz(v) => types::Value::TimestampTz(v),
        };
        Some(value)
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
    /// A [`ServerError`](crate::ErrorKind::ServerError) with the code `invalid`
    /// when `column` is absent from the row, holds `NULL` for a non-`Option`
    /// `T`, or has an incompatible [`Value`].
    pub fn get<'a, T: FromColumn<'a>>(&'a self, column: &str) -> Result<T> {
        let value = self
            .fields
            .iter()
            .find(|field| field.name == column)
            .map(|field| &field.value)
            .ok_or_else(|| column_missing(column))?;
        T::from_column(column, value)
    }

    fn at<'a, T: FromColumn<'a>>(&'a self, index: usize) -> Result<T> {
        let field = self.fields.get(index).ok_or_else(|| {
            crate::Error::server_error("invalid", format!("row lacks a column at index {index}"))
        })?;
        T::from_column(&field.name, &field.value)
    }
}

fn column_missing(column: &str) -> crate::Error {
    crate::Error::server_error("invalid", format!("row lacks column `{column}`"))
}

fn column_null(column: &str) -> crate::Error {
    crate::Error::server_error("invalid", format!("column `{column}` is null"))
}

fn column_mismatch(column: &str, expected: &str, actual: &Value) -> crate::Error {
    crate::Error::server_error(
        "invalid",
        format!("column `{column}` is not {expected}: {actual:?}"),
    )
}

fn column_overflow(column: &str, target: &str, err: impl fmt::Display) -> crate::Error {
    crate::Error::server_error("invalid", format!("column `{column}` overflows {target}: {err}"))
}

/// Decode a Rust value from one SQL column's [`Value`].
///
/// - Every integer case reads as any integer type its value fits.
/// - `Float32` and `Float64` read as `f64`, and only `Float32` reads as
///   `f32`.
/// - `Boolean` reads as `bool`, and so does an integer `0` or `1`, which is
///   how `SQLite` returns a `BOOLEAN` column.
/// - Every string-shaped case (`Text`, `Date`, `Time`, `Timestamp`,
///   `TimestampTz`) reads as a string.
/// - `Option<T>` reads `NULL` as `None`.
pub trait FromColumn<'a>: Sized {
    /// Decode `Self` from `value`, the content of `column`.
    ///
    /// # Errors
    ///
    /// A [`ServerError`](crate::ErrorKind::ServerError) with the code `invalid`
    /// when `value` is `NULL` for a non-`Option` `Self` or has an
    /// incompatible [`Value`].
    fn from_column(column: &str, value: &'a Value) -> Result<Self>;
}

impl<'a, T: FromColumn<'a>> FromColumn<'a> for Option<T> {
    fn from_column(column: &str, value: &'a Value) -> Result<Self> {
        if value.is_null() { Ok(None) } else { T::from_column(column, value).map(Some) }
    }
}

impl FromColumn<'_> for i64 {
    fn from_column(column: &str, value: &Value) -> Result<Self> {
        match value {
            Value::Int64(v) => Ok(*v),
            Value::Int32(v) => Ok(Self::from(*v)),
            Value::Null => Err(column_null(column)),
            other => Err(column_mismatch(column, "an integer", other)),
        }
    }
}

impl FromColumn<'_> for i32 {
    fn from_column(column: &str, value: &Value) -> Result<Self> {
        let wide = i64::from_column(column, value)?;
        Self::try_from(wide).map_err(|err| column_overflow(column, "i32", err))
    }
}

impl FromColumn<'_> for u64 {
    fn from_column(column: &str, value: &Value) -> Result<Self> {
        let wide = i64::from_column(column, value)?;
        Self::try_from(wide).map_err(|err| column_overflow(column, "u64", err))
    }
}

impl FromColumn<'_> for u32 {
    fn from_column(column: &str, value: &Value) -> Result<Self> {
        let wide = u64::from_column(column, value)?;
        Self::try_from(wide).map_err(|err| column_overflow(column, "u32", err))
    }
}

impl FromColumn<'_> for f64 {
    fn from_column(column: &str, value: &Value) -> Result<Self> {
        match value {
            Value::Float64(v) => Ok(*v),
            Value::Float32(v) => Ok(Self::from(*v)),
            Value::Null => Err(column_null(column)),
            other => Err(column_mismatch(column, "a float", other)),
        }
    }
}

impl FromColumn<'_> for f32 {
    fn from_column(column: &str, value: &Value) -> Result<Self> {
        match value {
            Value::Float32(v) => Ok(*v),
            Value::Null => Err(column_null(column)),
            other => Err(column_mismatch(column, "a 32-bit float", other)),
        }
    }
}

impl FromColumn<'_> for bool {
    fn from_column(column: &str, value: &Value) -> Result<Self> {
        match value {
            Value::Boolean(v) => Ok(*v),
            Value::Int32(0) | Value::Int64(0) => Ok(false),
            Value::Int32(1) | Value::Int64(1) => Ok(true),
            Value::Null => Err(column_null(column)),
            other => Err(column_mismatch(column, "a boolean", other)),
        }
    }
}

impl<'a> FromColumn<'a> for &'a str {
    fn from_column(column: &str, value: &'a Value) -> Result<Self> {
        match value {
            Value::Text(v)
            | Value::Date(v)
            | Value::Time(v)
            | Value::Timestamp(v)
            | Value::TimestampTz(v) => Ok(v.as_str()),
            Value::Null => Err(column_null(column)),
            other => Err(column_mismatch(column, "a string", other)),
        }
    }
}

impl<'a> FromColumn<'a> for String {
    fn from_column(column: &str, value: &'a Value) -> Result<Self> {
        <&'a str as FromColumn<'a>>::from_column(column, value).map(str::to_owned)
    }
}

impl<'a> FromColumn<'a> for &'a [u8] {
    fn from_column(column: &str, value: &'a Value) -> Result<Self> {
        match value {
            Value::Binary(v) => Ok(v.as_ref()),
            Value::Null => Err(column_null(column)),
            other => Err(column_mismatch(column, "binary bytes", other)),
        }
    }
}

impl<'a> FromColumn<'a> for Vec<u8> {
    fn from_column(column: &str, value: &'a Value) -> Result<Self> {
        <&'a [u8] as FromColumn<'a>>::from_column(column, value).map(<[u8]>::to_vec)
    }
}

impl FromColumn<'_> for Bytes {
    fn from_column(column: &str, value: &Value) -> Result<Self> {
        match value {
            Value::Binary(v) => Ok(v.clone()),
            Value::Null => Err(column_null(column)),
            other => Err(column_mismatch(column, "binary bytes", other)),
        }
    }
}

/// Decode a domain record from a SQL [`Row`].
///
/// Guest code usually derives this through `omnia_sdk::sql::FromRow`; a
/// hand-written impl reads each column through [`Row::get`]. A tuple of up to
/// twelve owned [`FromColumn`] types decodes its row by position, the first
/// column into the first element, and ignores any columns past its own.
pub trait FromRow: Sized {
    /// Decode `Self` from `row`.
    ///
    /// # Errors
    ///
    /// A [`ServerError`](crate::ErrorKind::ServerError) with the code `invalid`
    /// when a required column is missing, `NULL`, or of an incompatible
    /// [`Value`].
    fn from_row(row: &Row) -> Result<Self>;
}

impl FromRow for Row {
    fn from_row(row: &Row) -> Result<Self> {
        Ok(row.clone())
    }
}

macro_rules! tuple_from_row {
    ($($ty:ident $index:tt),+) => {
        impl<$($ty),+> FromRow for ($($ty,)+)
        where
            $($ty: for<'a> FromColumn<'a>,)+
        {
            fn from_row(row: &Row) -> Result<Self> {
                Ok(($(row.at::<$ty>($index)?,)+))
            }
        }
    };
}

tuple_from_row!(A 0);
tuple_from_row!(A 0, B 1);
tuple_from_row!(A 0, B 1, C 2);
tuple_from_row!(A 0, B 1, C 2, D 3);
tuple_from_row!(A 0, B 1, C 2, D 3, E 4);
tuple_from_row!(A 0, B 1, C 2, D 3, E 4, F 5);
tuple_from_row!(A 0, B 1, C 2, D 3, E 4, F 5, G 6);
tuple_from_row!(A 0, B 1, C 2, D 3, E 4, F 5, G 6, H 7);
tuple_from_row!(A 0, B 1, C 2, D 3, E 4, F 5, G 6, H 7, I 8);
tuple_from_row!(A 0, B 1, C 2, D 3, E 4, F 5, G 6, H 7, I 8, J 9);
tuple_from_row!(A 0, B 1, C 2, D 3, E 4, F 5, G 6, H 7, I 8, J 9, K 10);
tuple_from_row!(A 0, B 1, C 2, D 3, E 4, F 5, G 6, H 7, I 8, J 9, K 10, L 11);
