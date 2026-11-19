//! The pure helpers over the generated `omnia:*` records: TTLs, messages,
//! statement binding, row decoding, tokens, and JSON codecs. Every helper
//! that calls the host is a `test-programs` guest instead.

#![cfg(not(target_arch = "wasm32"))]

use std::collections::BTreeMap;
use std::time::Duration;

use omnia_sdk::docstore::types::Document;
use omnia_sdk::identity::AccessToken;
use omnia_sdk::keyvalue::types::Lifetime;
use omnia_sdk::keyvalue::{self};
use omnia_sdk::messaging::types::Message;
use omnia_sdk::sql::database::Statement;
use omnia_sdk::sql::types::Value as Wire;
use omnia_sdk::sql::{Field, FromRow, Row, Value};
use serde::Deserialize;
use serde_json::json;

#[test]
fn ttl_lifetime() {
    assert_eq!(keyvalue::ttl_ms(Duration::ZERO), 0);
    assert_eq!(keyvalue::ttl_ms(Duration::from_nanos(1)), 1, "a sub-millisecond TTL expires");
    assert_eq!(keyvalue::ttl_ms(Duration::from_micros(1_500)), 2);
    assert_eq!(keyvalue::ttl_ms(Duration::MAX), u64::MAX);

    assert!(matches!(Lifetime::from(None), Lifetime::Forever));
    assert!(matches!(Lifetime::from(Some(Duration::from_secs(2))), Lifetime::TtlMs(2_000)));
    assert!(matches!(Lifetime::from(Duration::ZERO), Lifetime::TtlMs(0)));
}

#[test]
fn message_json() {
    let message = Message::json(&json!({"id": 7}))
        .expect("serialize")
        .with_key("order-7")
        .with_metadata("trace", "abc");

    assert_eq!(message.data, br#"{"id":7}"#);
    assert_eq!(message.content_type.as_deref(), Some("application/json"));
    assert_eq!(message.key.as_deref(), Some("order-7"));
    assert_eq!(message.metadata, [("trace".to_owned(), "abc".to_owned())]);
}

#[test]
fn message_raw() {
    let message = Message::new("plain").with_content_type("text/plain");

    assert_eq!(message.data, b"plain");
    assert_eq!(message.content_type.as_deref(), Some("text/plain"));
    assert_eq!(message.key, None);
    assert_eq!(message.metadata, []);
}

#[test]
fn message_unserializable() {
    let keyed_by_pairs = BTreeMap::from([((1, 2), "pair")]);

    let error = Message::json(&keyed_by_pairs).expect_err("JSON keys are strings");

    assert_eq!(error.code(), "invalid");
}

#[test]
fn statement_binding() {
    let params = [
        Value::Null,
        Value::from(7_i64),
        Value::from("text"),
        Value::Timestamp("2024-01-02 03:04:05".to_owned()),
        Value::Timestamp("2024-01-02T03:04:05Z".to_owned()),
        Value::Timestamp("2024-01-02T03:04:05-05:00".to_owned()),
    ];

    let statement = Statement::new("INSERT INTO t VALUES ($1, $2, $3, $4, $5, $6)", &params);

    assert_eq!(statement.sql, "INSERT INTO t VALUES ($1, $2, $3, $4, $5, $6)");
    assert!(
        matches!(
            statement.params.as_slice(),
            [
                None,
                Some(Wire::Int64(7)),
                Some(Wire::Text(_)),
                Some(Wire::Timestamp(_)),
                Some(Wire::TimestampTz(_)),
                Some(Wire::TimestampTz(_)),
            ]
        ),
        "{:?}",
        statement.params
    );
}

#[test]
fn wire_value() {
    assert_eq!(Value::from(None::<Wire>), Value::Null);
    assert_eq!(Value::from(Wire::Float32(1.5)), Value::Float(1.5));
    assert_eq!(Value::from(Wire::Binary(vec![1, 2])), Value::from(vec![1_u8, 2]));
    let zoned = Value::from(Wire::TimestampTz("2024-01-02T03:04:05Z".to_owned()));
    assert_eq!(zoned, Value::Timestamp("2024-01-02T03:04:05Z".to_owned()));
}

#[test]
fn row_from_row() {
    let row = Row {
        fields: vec![Field {
            name: "id".to_owned(),
            value: Value::from(1_i64),
        }],
    };

    assert_eq!(Row::from_row(&row).expect("a row reads as itself"), row);
}

#[derive(Debug, PartialEq, Deserialize)]
struct Order {
    id: u32,
}

#[test]
fn document_decode() {
    let document = Document {
        id: "order-1".to_owned(),
        data: r#"{"id": 1}"#.to_owned(),
        revision: "r1".to_owned(),
    };

    assert_eq!(document.decode::<Order>().expect("decode"), Order { id: 1 });
    let error = document.decode::<Vec<u8>>().expect_err("an object is not a list");
    assert_eq!(error.code(), "invalid");
    assert!(error.description().contains("order-1"), "{error}");
}

#[test]
fn row_get_null() {
    let row = item(None);

    let note: Option<String> = row.get("note").expect("nullable column");
    let error = row.get::<String>("note").expect_err("NULL into String");

    assert_eq!(note, None);
    assert_eq!(error.description(), "column `note` is null");
}

#[test]
fn row_get_mismatch() {
    let row = item(None);

    let error = row.get::<i64>("label").expect_err("text into i64");
    let missing = row.get::<i64>("absent").expect_err("no such column");

    assert!(error.description().starts_with("column `label` is not an integer"), "{error}");
    assert_eq!(missing.description(), "row lacks column `absent`");
}

fn item(note: Option<&str>) -> Row {
    let field = |name: &str, value: Value| Field {
        name: name.to_owned(),
        value,
    };
    Row {
        fields: vec![
            field("id", 1_i64.into()),
            field("label", "alpha".into()),
            field("note", note.into()),
        ],
    }
}

#[test]
fn access_token_debug() {
    let token = AccessToken::new("secret", Duration::from_secs(60));

    let debug = format!("{token:?}");

    assert!(!debug.contains("secret"), "{debug}");
    assert!(debug.contains("<redacted>"), "{debug}");
}
