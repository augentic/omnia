//! Integration tests for the ORM `entity!` macro.
//!
//! Tests the public API as users would interact with it.

#![cfg(target_arch = "wasm32")]
#![allow(
    missing_docs,
    clippy::struct_field_names,
    clippy::approx_constant,
    clippy::float_cmp,
    clippy::too_many_lines
)]

mod common;

use chrono::{DateTime, Utc};
use common::{Event, User};
use omnia_orm::{DataType, Entity, Field, Row, entity};

use crate::common::Item;

#[test]
fn entity_basic() {
    assert_eq!(User::TABLE, "users");
    assert_eq!(User::COLUMNS, &["id", "name", "active"]);
}

#[test]
fn entity_from_row_field_missing() {
    let row = Row {
        fields: vec![Field {
            name: "id".to_string(),
            value: DataType::Int64(Some(1)),
        }],
        index: "0".to_string(),
    };

    let result = User::from_row(&row);
    assert!(result.is_err());
    let err = result.err().unwrap();
    assert!(err.to_string().contains("missing column"));
}

#[test]
fn entity_with_datetime_fallback_format() {
    let row = Row {
        fields: vec![
            Field {
                name: "id".to_string(),
                value: DataType::Int64(Some(1)),
            },
            Field {
                name: "occurred_at".to_string(),
                value: DataType::Timestamp(Some("2024-01-15 10:30:45.123".to_string())),
            },
        ],
        index: "0".to_string(),
    };

    let result = Event::from_row(&row).unwrap();
    assert_eq!(result.id, 1);
    assert_eq!(result.occurred_at.format("%Y-%m-%d %H:%M:%S").to_string(), "2024-01-15 10:30:45");
}

#[test]
fn entity_with_datetime_invalid_format() {
    let row = Row {
        fields: vec![
            Field {
                name: "id".to_string(),
                value: DataType::Int64(Some(1)),
            },
            Field {
                name: "occurred_at".to_string(),
                value: DataType::Timestamp(Some("not a valid date".to_string())),
            },
        ],
        index: "0".to_string(),
    };

    let result = Event::from_row(&row);
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("unsupported timestamp"));
}

#[test]
fn entity_from_row_field_type_mismatch() {
    let row = Row {
        fields: vec![
            Field {
                name: "id".to_string(),
                value: DataType::Int64(Some(1)),
            },
            Field {
                name: "count".to_string(),
                value: DataType::Str(Some("not_a_number".to_string())),
            },
        ],
        index: "0".to_string(),
    };

    let result = Item::from_row(&row);
    result.unwrap_err();
}

#[test]
fn entity_with_multiple_fields() {
    entity! {
        table = "test_records",
        pub struct TestRecords {
            pub bool_field: bool,
            pub i32_field: i32,
            pub i64_field: i64,
            pub u32_field: u32,
            pub u64_field: u64,
            pub f32_field: f32,
            pub f64_field: f64,
            pub string_field: String,
            pub bytes_field: Vec<u8>,
            pub json_field: Vec<u8>,
            pub dt_field: DateTime<Utc>,
        }
    }

    let row = Row {
        fields: vec![
            Field {
                name: "bool_field".to_string(),
                value: DataType::Boolean(Some(true)),
            },
            Field {
                name: "i32_field".to_string(),
                value: DataType::Int32(Some(42)),
            },
            Field {
                name: "i64_field".to_string(),
                value: DataType::Int64(Some(1000)),
            },
            Field {
                name: "u32_field".to_string(),
                value: DataType::Uint32(Some(100)),
            },
            Field {
                name: "u64_field".to_string(),
                value: DataType::Uint64(Some(2000)),
            },
            Field {
                name: "f32_field".to_string(),
                value: DataType::Float(Some(3.14)),
            },
            Field {
                name: "f64_field".to_string(),
                value: DataType::Double(Some(2.718)),
            },
            Field {
                name: "string_field".to_string(),
                value: DataType::Str(Some("test".to_string())),
            },
            Field {
                name: "bytes_field".to_string(),
                value: DataType::Binary(Some(vec![1, 2, 3])),
            },
            Field {
                name: "json_field".to_string(),
                value: DataType::Binary(Some(br#"{"key":"value","count":42}"#.to_vec())),
            },
            Field {
                name: "dt_field".to_string(),
                value: DataType::Timestamp(Some("2024-01-15T10:30:45Z".to_string())),
            },
        ],
        index: "0".to_string(),
    };

    let result = TestRecords::from_row(&row).unwrap();
    assert!(result.bool_field);
    assert_eq!(result.i32_field, 42);
    assert_eq!(result.i64_field, 1000);
    assert_eq!(result.u32_field, 100);
    assert_eq!(result.u64_field, 2000);
    assert_eq!(result.f32_field, 3.14);
    assert_eq!(result.f64_field, 2.718);
    assert_eq!(result.string_field, "test");
    assert_eq!(result.bytes_field, vec![1, 2, 3]);
    assert_eq!(result.json_field, br#"{"key":"value","count":42}"#.to_vec());
    assert_eq!(result.dt_field.format("%Y-%m-%d").to_string(), "2024-01-15");

    let json: serde_json::Value = serde_json::from_slice(&result.json_field).unwrap();
    assert_eq!(json["key"], "value");
    assert_eq!(json["count"], 42);
}

#[test]
fn entity_with_multiple_optional_fields() {
    entity! {
        table = "test_records",
        pub struct TestRecords {
            pub bool_field: Option<bool>,
            pub i32_field: Option<i32>,
            pub i64_field: Option<i64>,
            pub u32_field: Option<u32>,
            pub u64_field: Option<u64>,
            pub f32_field: Option<f32>,
            pub f64_field: Option<f64>,
            pub string_field: Option<String>,
            pub bytes_field: Option<Vec<u8>>,
            pub dt_field: Option<DateTime<Utc>>,
        }
    }

    let row = Row {
        fields: vec![
            Field {
                name: "bool_field".to_string(),
                value: DataType::Boolean(None),
            },
            Field {
                name: "i32_field".to_string(),
                value: DataType::Int32(None),
            },
            Field {
                name: "i64_field".to_string(),
                value: DataType::Int64(None),
            },
            Field {
                name: "u32_field".to_string(),
                value: DataType::Uint32(None),
            },
            Field {
                name: "u64_field".to_string(),
                value: DataType::Uint64(None),
            },
            Field {
                name: "f32_field".to_string(),
                value: DataType::Float(None),
            },
            Field {
                name: "f64_field".to_string(),
                value: DataType::Double(None),
            },
            Field {
                name: "string_field".to_string(),
                value: DataType::Str(None),
            },
            Field {
                name: "bytes_field".to_string(),
                value: DataType::Binary(None),
            },
            Field {
                name: "dt_field".to_string(),
                value: DataType::Timestamp(None),
            },
        ],
        index: "0".to_string(),
    };

    let result = TestRecords::from_row(&row).unwrap();
    assert_eq!(result.bool_field, None);
    assert_eq!(result.i32_field, None);
    assert_eq!(result.i64_field, None);
    assert_eq!(result.u32_field, None);
    assert_eq!(result.u64_field, None);
    assert_eq!(result.f32_field, None);
    assert_eq!(result.f64_field, None);
    assert_eq!(result.string_field, None);
    assert_eq!(result.bytes_field, None);
    assert_eq!(result.dt_field, None);
}
