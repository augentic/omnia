//! # Convert
//!
//! Convert between OpenTelemetry types and `omnia:otel` types.

use std::time::{SystemTime, UNIX_EPOCH};

use opentelemetry::{Array, InstrumentationScope, KeyValue, Value};

use crate::otel::types as wasi;

impl From<Value> for wasi::AnyValue {
    fn from(value: Value) -> Self {
        match value {
            Value::Bool(v) => Self::BoolValue(v),
            Value::I64(v) => Self::IntValue(v),
            Value::F64(v) => Self::DoubleValue(v),
            Value::String(v) => Self::StringValue(v.into()),
            Value::Array(Array::Bool(items)) => Self::BoolArray(items),
            Value::Array(Array::I64(items)) => Self::IntArray(items),
            Value::Array(Array::F64(items)) => Self::DoubleArray(items),
            Value::Array(Array::String(items)) => {
                Self::StringArray(items.into_iter().map(Into::into).collect())
            }
            // `Value` and `Array` are `#[non_exhaustive]`
            value => Self::StringValue(value.as_str().into_owned()),
        }
    }
}

impl From<KeyValue> for wasi::KeyValue {
    fn from(kv: KeyValue) -> Self {
        Self {
            key: kv.key.to_string(),
            value: kv.value.into(),
        }
    }
}

impl From<&KeyValue> for wasi::KeyValue {
    fn from(kv: &KeyValue) -> Self {
        Self {
            key: kv.key.to_string(),
            value: kv.value.clone().into(),
        }
    }
}

impl From<&InstrumentationScope> for wasi::InstrumentationScope {
    fn from(scope: &InstrumentationScope) -> Self {
        Self {
            name: scope.name().to_string(),
            version: scope.version().map(Into::into),
            attributes: scope.attributes().map(Into::into).collect(),
            dropped_attributes_count: 0,
        }
    }
}

// a time before the epoch reads as the epoch, and one past the year 2554
// saturates
pub fn unix_nanos(time: SystemTime) -> u64 {
    let since_epoch = time.duration_since(UNIX_EPOCH).unwrap_or_default();
    u64::try_from(since_epoch.as_nanos()).unwrap_or(u64::MAX)
}
