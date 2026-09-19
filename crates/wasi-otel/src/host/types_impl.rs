use opentelemetry::{Array, Key, Value};
use opentelemetry_sdk::Resource;

use crate::host::WasiOtelCtxView;
use crate::host::generated::omnia::otel::types;

impl types::Host for WasiOtelCtxView<'_> {
    fn convert_error(&mut self, err: types::Error) -> wasmtime::Result<types::Error> {
        tracing::error!("{err}");
        Ok(err)
    }
}

impl From<&Resource> for types::Resource {
    fn from(resource: &Resource) -> Self {
        Self {
            attributes: resource.iter().map(Into::into).collect(),
            schema_url: resource.schema_url().map(Into::into),
        }
    }
}

impl From<(&Key, &Value)> for types::KeyValue {
    fn from((key, value): (&Key, &Value)) -> Self {
        Self {
            key: key.to_string(),
            value: value.clone().into(),
        }
    }
}

impl From<Value> for types::Value {
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

/// Hex-decode a guest-supplied trace/span id, warning (rather than silently
/// substituting an empty id) when the id is malformed.
pub fn decode_id(id: &str) -> Vec<u8> {
    hex::decode(id).unwrap_or_else(|error| {
        tracing::warn!(%error, id, "malformed hex trace/span id; emitting empty id");
        Vec::new()
    })
}

pub fn datetime_nanos(dt: types::Datetime) -> u64 {
    // Saturate rather than overflow: a guest-supplied timestamp past the year
    // 2554 clamps to `u64::MAX` instead of panicking (debug) or wrapping
    // (release) into a bogus time.
    dt.seconds.saturating_mul(1_000_000_000).saturating_add(u64::from(dt.nanoseconds))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_id_hex() {
        assert_eq!(decode_id("0af7651916cd43dd"), [0x0a, 0xf7, 0x65, 0x19, 0x16, 0xcd, 0x43, 0xdd]);
    }

    #[test]
    fn decode_id_malformed() {
        assert!(decode_id("not-hex").is_empty());
        assert!(decode_id("abc").is_empty());
    }

    #[test]
    fn datetime_nanos_combines() {
        let dt = types::Datetime {
            seconds: 3,
            nanoseconds: 7,
        };
        assert_eq!(datetime_nanos(dt), 3_000_000_007);
    }

    #[test]
    fn datetime_nanos_saturates() {
        let dt = types::Datetime {
            seconds: u64::MAX,
            nanoseconds: 999_999_999,
        };
        assert_eq!(datetime_nanos(dt), u64::MAX);
    }
}
