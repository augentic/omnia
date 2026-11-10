//! Typed capability helper contracts over the `omnia_test::guest` doubles.

#![cfg(all(not(target_arch = "wasm32"), feature = "sql", feature = "docstore"))]

use bytes::Bytes;
use http::{Method, Request, Response, StatusCode};
use http_body_util::Full;
use omnia_sdk::docstore::Document;
use omnia_sdk::sql::{DataType, Field, FromRow, Row};
use omnia_sdk::{
    ConfigExt as _, DocumentStore as _, DocumentStoreExt as _, Error, HttpRequestExt as _,
    IdentityExt as _, Message, Publish as _, Settings, StateStoreExt as _, TableStoreExt as _,
};
use omnia_test::guest::{
    FixedIdentity, MapConfig, MatchedHttp, MemoryDocs, Provider, ScriptedTables, Sink,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

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
        index: id.to_string(),
        fields: vec![
            Field {
                name: "id".into(),
                value: DataType::Int64(Some(id)),
            },
            Field {
                name: "label".into(),
                value: DataType::Str(Some(label.into())),
            },
            Field {
                name: "note".into(),
                value: DataType::Str(note.map(str::to_owned)),
            },
        ],
    }
}

#[tokio::test]
async fn state_json_round_trip() {
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
async fn fetch_json_upstream_ok() {
    let http = MatchedHttp::default().on(
        Method::GET,
        "https://api.test/status",
        Response::new(Bytes::from_static(br#"{"status":"green"}"#)),
    );
    let request = Request::get("https://api.test/status").body(Full::<Bytes>::default()).unwrap();

    let upstream: Upstream = http.fetch_json(request).await.expect("fetch");

    assert_eq!(upstream.status, "green");
}

#[tokio::test]
async fn fetch_json_upstream_failure() {
    let mut unavailable = Response::new(Bytes::from_static(b"try later"));
    *unavailable.status_mut() = StatusCode::SERVICE_UNAVAILABLE;
    let http = MatchedHttp::default().on(Method::GET, "https://api.test/status", unavailable);
    let request = Request::get("https://api.test/status").body(Full::<Bytes>::default()).unwrap();

    let error = http.fetch_json::<_, Upstream>(request).await.expect_err("non-2xx fails");

    assert!(matches!(error, Error::BadGateway { .. }), "{error}");
    assert!(error.to_string().contains("503"), "{error}");
}

#[tokio::test]
async fn config_get_parsed_invalid() {
    let config = MapConfig::default().with([("PORT", "eighty")]);

    let error = config.get_parsed::<u16>("PORT").await.expect_err("not a number");

    assert!(error.to_string().contains("`PORT` is invalid"), "{error}");
}

#[tokio::test]
async fn identity_access_token() {
    let identity = FixedIdentity::new("tok");

    let token = identity.access_token("svc").await.expect("token");

    assert_eq!(token, "tok");
    assert_eq!(identity.asked(), ["svc"]);
}

#[tokio::test]
async fn document_json_round_trip() {
    let docs = MemoryDocs::default();
    let value = Cached {
        name: "ann".into(),
        hits: 2,
    };

    let document = Document::json("ann", &value).expect("encode");
    docs.put("people", &document).await.expect("put");
    let stored: Option<Cached> = docs.get_as("people", "ann").await.expect("get");
    let absent: Option<Cached> = docs.get_as("people", "bob").await.expect("get absent");

    assert_eq!(document.data, br#"{"name":"ann","hits":2}"#);
    assert_eq!(stored, Some(value));
    assert_eq!(absent, None);
    let reached = docs.documents("people").await.expect("documents");
    assert_eq!(reached.len(), 1, "{reached:?}");
    assert_eq!(reached[0].id, "ann");
    assert_eq!(
        serde_json::from_slice::<Value>(&reached[0].data).expect("JSON body"),
        json!({"name": "ann", "hits": 2})
    );
}

#[tokio::test]
async fn document_decode_invalid() {
    let docs = MemoryDocs::default();
    let document = Document {
        id: "bob".into(),
        data: br#"{"name":"bob"}"#.to_vec(),
    };
    docs.put("people", &document).await.expect("put");

    let error = docs.get_as::<Cached>("people", "bob").await.expect_err("no `hits` field");

    assert_eq!(error.to_string(), "decoding document `bob`");
    assert!(format!("{error:#}").contains("missing field `hits`"), "{error:#}");
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
        .query_as(
            "db",
            "SELECT id, label, note FROM items WHERE id > $1",
            &[DataType::Int64(Some(0))],
        )
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
    assert_eq!(tables.statements()[0].connection, "db");
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
fn row_get_type_mismatch() {
    let row = row(1, "alpha", None);

    let error = row.get::<i64>("label").expect_err("text into i64");
    let missing = row.get::<i64>("absent").expect_err("no such column");

    assert!(error.description().starts_with("column `label` is not an integer"), "{error}");
    assert_eq!(missing.description(), "row lacks column `absent`");
}

#[tokio::test]
async fn message_json_with_header() {
    let sink = Sink::default();
    let message =
        Message::json(&json!({"id": 7})).expect("serialize").with_header("tenant", "acme");

    sink.send("orders", &message).await.expect("send");

    let (topic, sent) = &sink.sent()[0];
    assert_eq!(topic, "orders");
    assert_eq!(sent.payload, br#"{"id":7}"#);
    assert_eq!(sent.headers.get("tenant").map(String::as_str), Some("acme"));
}
