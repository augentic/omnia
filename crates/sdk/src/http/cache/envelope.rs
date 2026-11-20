//! The stored form of a cached response.

use anyhow::{Context, Result};
use bytes::Bytes;
use http::Response;
use serde::{Deserialize, Serialize};

pub fn serialize(response: &Response<Bytes>) -> Result<Vec<u8>> {
    serde_json::to_vec(&Serialized::from(response)).context("serializing response")
}

pub fn deserialize(data: &[u8]) -> Result<Response<Bytes>> {
    let ser: Serialized = serde_json::from_slice(data).context("deserializing cached response")?;
    Response::<Bytes>::try_from(ser)
}

#[derive(Deserialize, Serialize)]
struct Serialized {
    status: u16,
    // raw bytes, since a field value may hold `obs-text` (RFC 9110 §5.5) and a
    // cache keeps every field it receives (RFC 9111 §3.1)
    headers: Vec<(String, Vec<u8>)>,
    body: Vec<u8>,
}

impl From<&Response<Bytes>> for Serialized {
    fn from(response: &Response<Bytes>) -> Self {
        Self {
            status: response.status().as_u16(),
            headers: response
                .headers()
                .iter()
                .map(|(k, v)| (k.to_string(), v.as_bytes().to_vec()))
                .collect(),
            body: response.body().to_vec(),
        }
    }
}

impl TryFrom<Serialized> for Response<Bytes> {
    type Error = anyhow::Error;

    fn try_from(s: Serialized) -> Result<Self> {
        let mut response = Response::builder().status(s.status);
        for (k, v) in s.headers {
            response = response.header(k, v);
        }
        response.body(Bytes::from(s.body)).context("building response from cached data")
    }
}
