use omnia_core::HostView;
use opentelemetry_proto::tonic::common::v1::any_value::Value;
use opentelemetry_proto::tonic::common::v1::{
    AnyValue, ArrayValue, InstrumentationScope, KeyValue, KeyValueList,
};
use opentelemetry_proto::tonic::resource::v1::Resource;

use crate::host::generated::omnia::otel::types;
use crate::host::{Error, OtelClient, Result};

impl types::Host for HostView<'_, dyn OtelClient> {
    fn convert_error(&mut self, err: Error) -> wasmtime::Result<Error> {
        Ok(err)
    }
}

impl Error {
    // the item an `invalid` message is about, in front of it
    pub(crate) fn within(self, item: &str) -> Self {
        match self {
            Self::Invalid(message) => Self::Invalid(format!("{item}: {message}")),
            err => err,
        }
    }
}

// the deployment's resource and its schema URL, `None` with no telemetry
// installed
pub fn resource() -> Option<(Resource, String)> {
    let resource = omnia_core::telemetry::resource()?;
    let schema_url = resource.schema_url().map_or_default(Into::into);
    let resource = Resource {
        attributes: resource.iter().map(Into::into).collect(),
        ..Resource::default()
    };
    Some((resource, schema_url))
}

pub fn trace_id(id: &str, field: &str) -> Result<Vec<u8>> {
    hex_id(id, 32, field)
}

pub fn span_id(id: &str, field: &str) -> Result<Vec<u8>> {
    hex_id(id, 16, field)
}

// `field` names the id in the `invalid` message
fn hex_id(id: &str, digits: usize, field: &str) -> Result<Vec<u8>> {
    if id.len() != digits || !id.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
        return Err(Error::Invalid(format!(
            "{field} `{id}` is not {digits} lowercase hexadecimal digits"
        )));
    }
    if id.bytes().all(|b| b == b'0') {
        return Err(Error::Invalid(format!("{field} is all zeros")));
    }
    hex::decode(id).map_err(|err| Error::Invalid(format!("{field} `{id}`: {err}")))
}

pub fn attributes(attributes: Vec<types::KeyValue>) -> Result<Vec<KeyValue>> {
    attributes.into_iter().map(KeyValue::try_from).collect()
}

impl TryFrom<types::KeyValue> for KeyValue {
    type Error = Error;

    fn try_from(attribute: types::KeyValue) -> Result<Self> {
        let value = AnyValue::try_from(attribute.value)
            .map_err(|err| err.within(&format!("attribute `{}`", attribute.key)))?;
        Ok(Self {
            key: attribute.key,
            value: Some(value),
            ..Self::default()
        })
    }
}

impl TryFrom<types::AnyValue> for AnyValue {
    type Error = Error;

    fn try_from(value: types::AnyValue) -> Result<Self> {
        let value = match value {
            types::AnyValue::StringValue(v) => Value::StringValue(v),
            types::AnyValue::BoolValue(v) => Value::BoolValue(v),
            types::AnyValue::IntValue(v) => Value::IntValue(v),
            types::AnyValue::DoubleValue(v) => Value::DoubleValue(v),
            types::AnyValue::BytesValue(v) => Value::BytesValue(v),
            types::AnyValue::StringArray(items) => array(items, Value::StringValue),
            types::AnyValue::BoolArray(items) => array(items, Value::BoolValue),
            types::AnyValue::IntArray(items) => array(items, Value::IntValue),
            types::AnyValue::DoubleArray(items) => array(items, Value::DoubleValue),
            types::AnyValue::JsonValue(text) => {
                let json = serde_json::from_str(&text)
                    .map_err(|err| Error::Invalid(format!("`json-value` is not JSON: {err}")))?;
                return Ok(from_json(json));
            }
            types::AnyValue::Empty => return Ok(Self { value: None }),
        };
        Ok(Self { value: Some(value) })
    }
}

fn array<T>(items: Vec<T>, value: fn(T) -> Value) -> Value {
    let values = items
        .into_iter()
        .map(|item| AnyValue {
            value: Some(value(item)),
        })
        .collect();
    Value::ArrayValue(ArrayValue { values })
}

// the reading `json-value` defines: an object is a map, `null` is the empty
// value, and a number is an integer only when it has no fraction or exponent
// and fits an `i64`
fn from_json(json: serde_json::Value) -> AnyValue {
    let value = match json {
        serde_json::Value::Null => return AnyValue { value: None },
        serde_json::Value::Bool(v) => Value::BoolValue(v),
        serde_json::Value::Number(n) => n
            .as_i64()
            .map_or_else(|| Value::DoubleValue(n.as_f64().unwrap_or(f64::NAN)), Value::IntValue),
        serde_json::Value::String(v) => Value::StringValue(v),
        serde_json::Value::Array(items) => Value::ArrayValue(ArrayValue {
            values: items.into_iter().map(from_json).collect(),
        }),
        serde_json::Value::Object(map) => Value::KvlistValue(KeyValueList {
            values: map
                .into_iter()
                .map(|(key, value)| KeyValue {
                    key,
                    value: Some(from_json(value)),
                    ..KeyValue::default()
                })
                .collect(),
        }),
    };
    AnyValue { value: Some(value) }
}

impl TryFrom<types::InstrumentationScope> for InstrumentationScope {
    type Error = Error;

    fn try_from(scope: types::InstrumentationScope) -> Result<Self> {
        let attributes = attributes(scope.attributes)
            .map_err(|err| err.within(&format!("scope `{}`", scope.name)))?;
        Ok(Self {
            name: scope.name,
            version: scope.version.unwrap_or_default(),
            attributes,
            dropped_attributes_count: scope.dropped_attributes_count,
        })
    }
}
