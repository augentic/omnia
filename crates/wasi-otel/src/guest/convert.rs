//! # Convert
//!
//! Convert between OpenTelemetry types and `wasi-otel` types.

use std::time::{SystemTime, UNIX_EPOCH};

use opentelemetry::{Array, InstrumentationScope, Key, KeyValue, Value};
use opentelemetry_sdk::Resource;

use crate::guest::generated::omnia::otel::types as wasi;
use crate::guest::generated::wasi::clocks::wall_clock::Datetime;

impl From<wasi::Resource> for Resource {
    fn from(value: wasi::Resource) -> Self {
        let attrs = value.attributes.into_iter().map(Into::into).collect::<Vec<_>>();
        let builder = Self::builder();

        if let Some(schema_url) = value.schema_url {
            builder.with_schema_url(attrs, schema_url).build()
        } else {
            builder.with_attributes(attrs).build()
        }
    }
}

impl From<wasi::KeyValue> for KeyValue {
    fn from(value: wasi::KeyValue) -> Self {
        Self::new(value.key, value.value)
    }
}

impl From<wasi::Value> for Value {
    fn from(value: wasi::Value) -> Self {
        match value {
            wasi::Value::Bool(v) => Self::Bool(v),
            wasi::Value::S64(v) => Self::I64(v),
            wasi::Value::F64(v) => Self::F64(v),
            wasi::Value::String(v) => Self::String(v.into()),
            wasi::Value::BoolArray(items) => Self::Array(Array::Bool(items)),
            wasi::Value::S64Array(items) => Self::Array(Array::I64(items)),
            wasi::Value::F64Array(items) => Self::Array(Array::F64(items)),
            wasi::Value::StringArray(items) => {
                Self::Array(Array::String(items.into_iter().map(Into::into).collect()))
            }
        }
    }
}

impl From<Value> for wasi::Value {
    fn from(value: Value) -> Self {
        match value {
            Value::Bool(v) => Self::Bool(v),
            Value::I64(v) => Self::S64(v),
            Value::F64(v) => Self::F64(v),
            Value::String(v) => Self::String(v.to_string()),
            Value::Array(v) => match v {
                Array::Bool(items) => Self::BoolArray(items),
                Array::I64(items) => Self::S64Array(items),
                Array::F64(items) => Self::F64Array(items),
                Array::String(items) => {
                    Self::StringArray(items.into_iter().map(Into::into).collect())
                }
                _ => Self::String(format!("{v:?}")),
            },
            _ => Self::String(format!("{value:?}")),
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
        kv.clone().into()
    }
}

impl From<(&Key, &Value)> for wasi::KeyValue {
    fn from((key, value): (&Key, &Value)) -> Self {
        Self {
            key: key.to_string(),
            value: value.clone().into(),
        }
    }
}

impl From<&Value> for wasi::Value {
    fn from(value: &Value) -> Self {
        value.clone().into()
    }
}

impl From<InstrumentationScope> for wasi::InstrumentationScope {
    fn from(scope: InstrumentationScope) -> Self {
        Self {
            name: scope.name().to_string(),
            version: scope.version().map(Into::into),
            schema_url: scope.schema_url().map(Into::into),
            attributes: scope.attributes().map(Into::into).collect(),
        }
    }
}

impl From<&InstrumentationScope> for wasi::InstrumentationScope {
    fn from(scope: &InstrumentationScope) -> Self {
        scope.clone().into()
    }
}

impl From<SystemTime> for Datetime {
    fn from(st: SystemTime) -> Self {
        let since_epoch = st.duration_since(UNIX_EPOCH).unwrap_or_default();
        Self {
            seconds: since_epoch.as_secs(),
            nanoseconds: since_epoch.subsec_nanos(),
        }
    }
}
