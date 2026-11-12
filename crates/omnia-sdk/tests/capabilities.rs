//! Typed capability helper contracts over the `omnia_test::guest` doubles.

#![cfg(not(target_arch = "wasm32"))]

use std::error::Error as _;

use bytes::Bytes;
use http::{Method, Request, Response, StatusCode};
use omnia_sdk::capability::ErrorKind;
use omnia_sdk::docstore::Document;
use omnia_sdk::messaging::Message;
use omnia_sdk::sql::{Field, FromRow, Row, Value};
use omnia_sdk::{
    ConfigExt as _, DocumentDb as _, DocumentDbExt as _, Error, HttpClientExt as _, Identity as _,
    IdentityExt as _, Publisher as _, Settings, SqlDb as _, SqlDbExt as _, StateStoreExt as _,
};
use omnia_test::guest::{
    FixedIdentity, MapConfig, MatchedHttp, MemoryDocs, Provider, ScriptedTables, Sink,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value as Json, json};

#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct Cached {
    name: String,
    hits: u32,
}

#[derive(Debug, Deserialize)]
struct Upstream {
    status: String,
}

#[derive(Debug, Settings)]
#[setting(prefix = "ORDERS_")]
struct OrdersSettings {
    upstream_url: String,
    #[setting(key = "RETRIES")]
    retries: u8,
}

#[derive(Debug, PartialEq, FromRow)]
struct Item {
    id: i64,
    label: String,
    note: Option<String>,
}

fn row(id: i64, label: &str, note: Option<&str>) -> Row {
    Row {
        fields: vec![
            Field {
                name: "id".into(),
                value: id.into(),
            },
            Field {
                name: "label".into(),
                value: label.into(),
            },
            Field {
                name: "note".into(),
                value: note.into(),
            },
        ],
    }
}

#[tokio::test]
async fn state_json() {
    let provider = Provider::default();
    let value = Cached {
        name: "ann".into(),
        hits: 2,
    };

    provider.set_json("cache:ann", &value, None).await.expect("set");
    let stored: Option<Cached> = provider.get_json("cache:ann").await.expect("get");
    let absent: Option<Cached> = provider.get_json("cache:bob").await.expect("get absent");

    assert_eq!(stored, Some(value));
    assert_eq!(
        provider.storage.state("cache:ann").as_deref(),
        Some(br#"{"name":"ann","hits":2}"#.as_slice())
    );
    assert_eq!(absent, None);
}

#[tokio::test]
async fn fetch_json() {
    let http = MatchedHttp::default().on(
        Method::GET,
        "https://api.test/status",
        Response::new(Bytes::from_static(br#"{"status":"green"}"#)),
    );
    let request = Request::get("https://api.test/status").body(Bytes::new()).unwrap();

    let upstream: Upstream = http.fetch_json(request).await.expect("fetch");

    assert_eq!(upstream.status, "green");
}

#[tokio::test]
async fn fetch_json_unavailable() {
    let mut unavailable = Response::new(Bytes::from_static(b"try later"));
    *unavailable.status_mut() = StatusCode::SERVICE_UNAVAILABLE;
    let http = MatchedHttp::default().on(Method::GET, "https://api.test/status", unavailable);
    let request = Request::get("https://api.test/status").body(Bytes::new()).unwrap();

    let error = http.fetch_json::<Upstream>(request).await.expect_err("non-2xx fails");

    assert!(matches!(error, Error::BadGateway { .. }), "{error}");
    assert!(error.to_string().contains("503"), "{error}");
}

#[tokio::test]
async fn config_require() {
    let config = MapConfig::default().with([("PORT", "eighty")]);

    let invalid = config.require::<u16>("PORT").await.expect_err("not a number");
    let unset = config.require::<u16>("HOST").await.expect_err("not set");
    let optional = config.parse::<u16>("HOST").await.expect("unset is not an error");

    assert_eq!(invalid.kind(), ErrorKind::Invalid);
    assert!(invalid.to_string().contains("`PORT` is invalid"), "{invalid}");
    assert_eq!(unset.kind(), ErrorKind::NotFound);
    assert_eq!(optional, None);
}

#[tokio::test]
async fn identity_access_token() {
    let identity = FixedIdentity::new("tok");

    let token = identity.named("svc").access_token().await.expect("token");

    assert_eq!(token, "tok:svc");
}

#[test]
fn access_token_debug() {
    let token = omnia_sdk::identity::AccessToken::new("secret", std::time::Duration::from_secs(60));

    let debug = format!("{token:?}");

    assert!(!debug.contains("secret"), "{debug}");
    assert!(debug.contains("<redacted>"), "{debug}");
}

#[tokio::test]
async fn document_json() {
    let docs = MemoryDocs::default();
    let value = Cached {
        name: "ann".into(),
        hits: 2,
    };

    let document = Document::json("ann", &value).expect("encode");
    docs.put("people", &document).await.expect("put");
    let stored: Option<Cached> = docs.get_as("people", "ann").await.expect("get");
    let absent: Option<Cached> = docs.get_as("people", "bob").await.expect("get absent");

    assert_eq!(&document.data[..], br#"{"name":"ann","hits":2}"#);
    assert_eq!(stored, Some(value));
    assert_eq!(absent, None);
    let reached = docs.documents("people").await.expect("documents");
    assert_eq!(reached.len(), 1, "{reached:?}");
    assert_eq!(reached[0].id, "ann");
    assert_eq!(
        serde_json::from_slice::<Json>(&reached[0].data).expect("JSON body"),
        json!({"name": "ann", "hits": 2})
    );
}

#[tokio::test]
async fn document_decode_invalid() {
    let docs = MemoryDocs::default();
    let document = Document::new("bob", br#"{"name":"bob"}"#);
    docs.put("people", &document).await.expect("put");

    let error = docs.get_as::<Cached>("people", "bob").await.expect_err("no `hits` field");

    assert_eq!(error.kind(), ErrorKind::Invalid);
    assert!(error.to_string().contains("decoding document `bob`"), "{error}");
    let source = error.source().map(ToString::to_string).unwrap_or_default();
    assert!(source.contains("missing field `hits`"), "{source}");
}

#[tokio::test]
async fn settings_load() {
    let provider = Provider::default().config(
        MapConfig::default()
            .with([("ORDERS_UPSTREAM_URL", "https://orders.test"), ("RETRIES", "3")]),
    );

    let settings = OrdersSettings::load(&provider).await.expect("load");

    assert_eq!(OrdersSettings::KEYS, ["ORDERS_UPSTREAM_URL", "RETRIES"]);
    assert_eq!(settings.upstream_url, "https://orders.test");
    assert_eq!(settings.retries, 3);
}

#[tokio::test]
async fn settings_missing_key() {
    let provider =
        Provider::default().config(MapConfig::default().with([("ORDERS_UPSTREAM_URL", "u")]));

    let error = OrdersSettings::load(&provider).await.expect_err("RETRIES is unset");

    assert!(matches!(error, Error::ServerError { .. }), "{error}");
    assert!(error.to_string().contains("RETRIES"), "{error}");
}

#[tokio::test]
async fn query_as() {
    let tables = ScriptedTables::default().on_query(
        |sql, params| sql.starts_with("SELECT") && params.len() == 1,
        vec![row(1, "alpha", None), row(2, "beta", Some("second"))],
    );

    let items: Vec<Item> = tables
        .named("orders")
        .query_as("SELECT id, label, note FROM items WHERE id > $1", &[Value::from(0_i64)])
        .await
        .expect("query");

    assert_eq!(
        items,
        [
            Item {
                id: 1,
                label: "alpha".into(),
                note: None,
            },
            Item {
                id: 2,
                label: "beta".into(),
                note: Some("second".into()),
            },
        ]
    );
    assert_eq!(tables.statements()[0].binding, "orders");
}

#[tokio::test]
async fn query_as_mismatch() {
    #[derive(Debug, FromRow)]
    struct Numbered {
        #[expect(dead_code, reason = "the row never decodes")]
        label: i64,
    }

    let tables = ScriptedTables::default().on_query(|_, _| true, vec![row(1, "alpha", None)]);

    let error =
        tables.query_as::<Numbered>("SELECT label FROM items", &[]).await.expect_err("text");

    assert_eq!(error.kind(), ErrorKind::Invalid);
}

#[test]
fn row_get_null() {
    let row = row(1, "alpha", None);

    let note: Option<String> = row.get("note").expect("nullable column");
    let error = row.get::<String>("note").expect_err("NULL into String");

    assert_eq!(note, None);
    assert_eq!(error.description(), "column `note` is null");
}

#[test]
fn row_get_mismatch() {
    let row = row(1, "alpha", None);

    let error = row.get::<i64>("label").expect_err("text into i64");
    let missing = row.get::<i64>("absent").expect_err("no such column");

    assert!(error.description().starts_with("column `label` is not an integer"), "{error}");
    assert_eq!(missing.description(), "row lacks column `absent`");
}

#[tokio::test]
async fn message_json() {
    let sink = Sink::default();
    let message = Message::json(&json!({"id": 7}))
        .expect("serialize")
        .with_key("order-7")
        .with_header("tenant", "acme");

    sink.publish("orders", &message).await.expect("publish");

    let (topic, sent) = &sink.sent()[0];
    assert_eq!(topic, "orders");
    assert_eq!(&sent.payload[..], br#"{"id":7}"#);
    assert_eq!(sent.key.as_deref(), Some("order-7"));
    assert_eq!(sent.content_type.as_deref(), Some("application/json"));
    assert_eq!(sent.headers.get("tenant").map(String::as_str), Some("acme"));
}
