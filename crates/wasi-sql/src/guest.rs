//! # WASI SQL WIT implementation

#![allow(clippy::same_length_and_capacity)]

// Bindings for the `wasi:sql` world.
// See (<https://github.com/augentic/wasi-sql/>)
wit_bindgen::generate!({
    world: "sql",
    path: "wit",
    generate_all,
});

use anyhow::Result;
use base64ct::{Base64, Encoding};
use serde_json::Value;

pub use self::wasi::sql::*;
use crate::types::{DataType, Row};

/// Helper function to create JSON output from rows returned by a query.
///
/// # Errors
/// Transforms into JSON value types fail.
pub fn into_json(rows: Vec<Row>) -> Result<Value> {
    let json_rows: Vec<Value> = rows
        .into_iter()
        .map(|row| {
            let mut map = serde_json::Map::new();
            for field in row.fields {
                let json_value = match field.value {
                    DataType::Int32(Some(v)) => Value::Number(v.into()),
                    DataType::Int64(Some(v)) => Value::Number(v.into()),
                    DataType::Uint32(Some(v)) => Value::Number(v.into()),
                    DataType::Uint64(Some(v)) => Value::Number(v.into()),
                    DataType::Float(Some(v)) => serde_json::Number::from_f64(f64::from(v))
                        .map_or(Value::Null, Value::Number),
                    DataType::Double(Some(v)) => {
                        serde_json::Number::from_f64(v).map_or(Value::Null, Value::Number)
                    }
                    DataType::Str(Some(v)) => Value::String(v),
                    DataType::Boolean(Some(v)) => Value::Bool(v),
                    DataType::Date(Some(formatted))
                    | DataType::Time(Some(formatted))
                    | DataType::Timestamp(Some(formatted)) => Value::String(formatted.value),
                    DataType::Binary(Some(v)) => {
                        let encoded = Base64::encode_string(&v);
                        Value::String(encoded)
                    }
                    DataType::Int32(None)
                    | DataType::Int64(None)
                    | DataType::Uint32(None)
                    | DataType::Uint64(None)
                    | DataType::Float(None)
                    | DataType::Double(None)
                    | DataType::Str(None)
                    | DataType::Boolean(None)
                    | DataType::Date(None)
                    | DataType::Time(None)
                    | DataType::Timestamp(None)
                    | DataType::Binary(None) => Value::Null,
                };
                map.insert(field.name, json_value);
            }
            Value::Object(map)
        })
        .collect();

    Ok(Value::Array(json_rows))
}
