//! Relational SQL service contract, value types, and default implementation.

#[cfg(not(target_arch = "wasm32"))]
mod client;
#[cfg(all(not(target_arch = "wasm32"), feature = "sqlite"))]
mod sqlite;

#[cfg(not(target_arch = "wasm32"))]
pub use self::client::{Connection, SqlClient};
#[cfg(all(not(target_arch = "wasm32"), feature = "sqlite"))]
pub use self::sqlite::{ConnectOptions, SqlDefault};

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
