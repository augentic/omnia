//! Conformance suite for `omnia_test::guest::Provider`.

#![cfg(not(target_arch = "wasm32"))]

use std::path::Path;

use bytes::Bytes;
use http::{HeaderValue, Method, Response};
use omnia_sdk::sql::{DataType, Field, Row};
use omnia_sdk::vcs;
use omnia_test::conformance;
use omnia_test::guest::{
    Broadcasted, FixedIdentity, MapConfig, MatchedHttp, Provider, ScriptedModel, ScriptedTables,
    ScriptedVcs, VcsTurn,
};

omnia_test::conformance!();

#[tokio::test]
async fn state_get_set_delete() {
    conformance::state_get_set_delete(&Provider::default()).await;
}

#[tokio::test]
async fn state_ttl_expires() {
    conformance::state_ttl_expires(&Provider::default()).await;
}

#[tokio::test]
async fn state_cas() {
    conformance::state_cas(&Provider::default()).await;
}

#[tokio::test]
async fn state_increment() {
    conformance::state_increment(&Provider::default()).await;
}

#[tokio::test]
async fn table_query_exec() {
    let row = Row {
        index: "0".to_owned(),
        fields: vec![
            Field {
                name: "id".to_owned(),
                value: DataType::Int64(Some(1)),
            },
            Field {
                name: "label".to_owned(),
                value: DataType::Str(Some("alpha".to_owned())),
            },
        ],
    };
    let tables = ScriptedTables::default()
        .on_exec(|sql, _| sql.starts_with("CREATE TABLE"), 0)
        .on_exec(|sql, params| sql.starts_with("INSERT INTO") && params.len() == 2, 1)
        .on_query(|sql, params| sql.starts_with("SELECT") && params.len() == 1, vec![row]);
    let provider = Provider::default().tables(tables);
    conformance::table_query_exec(&provider).await;

    let statements = provider.tables.statements();
    assert_eq!(statements.len(), 3);
    assert_eq!(statements[0].connection, "conformance");
    assert!(statements[1].sql.starts_with("INSERT INTO conformance_items"));
    assert_eq!(statements[2].params, [DataType::Int64(Some(1))]);
}

#[tokio::test]
async fn docstore_crud_query() {
    conformance::docstore_crud_query(&Provider::default()).await;
}

#[tokio::test]
async fn blobstore_objects() {
    conformance::blobstore_objects(&Provider::default()).await;
}

#[tokio::test]
async fn publish_send() {
    let provider = Provider::default();
    conformance::publish_send(&provider).await;
    let sent = provider.publish.sent();
    let [(topic, message)] = sent.as_slice() else {
        panic!("one message published, not {}", sent.len());
    };
    assert_eq!(topic, "conformance.events");
    assert_eq!(message.payload, b"conformance-event");
    assert_eq!(message.headers.get("x-trace").map(String::as_str), Some("1"));
}

#[tokio::test]
async fn config_get() {
    let provider = Provider::default()
        .config(MapConfig::default().with([("CONFORMANCE_KEY", "conformance-val")]));
    conformance::config_get(&provider).await;
}

#[tokio::test]
async fn identity_token() {
    let provider = Provider::default().identity(FixedIdentity::new("conformance-token"));
    conformance::identity_token(&provider).await;
}

#[tokio::test]
async fn vault_locker() {
    conformance::vault_locker(&Provider::default()).await;
}

#[tokio::test]
async fn http_fetch() {
    let http = MatchedHttp::default().on(
        Method::GET,
        "http://origin.test/conformance",
        Response::new(Bytes::from_static(b"conformance-ok")),
    );
    let provider = Provider::default().http(http);
    conformance::http_fetch(&provider, "http://origin.test").await;

    let requests = provider.http.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].uri.path(), "/conformance");
    assert_eq!(requests[0].headers.get("x-probe").map(HeaderValue::as_bytes), Some(&b"1"[..]));
}

#[tokio::test]
async fn broadcast_send() {
    let provider = Provider::default();
    conformance::broadcast_send(&provider).await;
    assert_eq!(
        provider.broadcast.broadcasts(),
        [Broadcasted {
            name: "conformance".to_owned(),
            data: b"ping".to_vec(),
            sockets: None,
        }]
    );
}

#[tokio::test]
async fn model_complete() {
    let provider = Provider::default().model(ScriptedModel::answering(["conformance", "checked"]));
    conformance::model_complete(&provider).await;
    provider.model.assert_exhausted();

    let seen = provider.model.seen();
    assert_eq!(seen.len(), 2);
    assert_eq!(
        (seen[0].messages.as_slice(), seen[0].check),
        (&["conformance".to_owned()][..], false)
    );
    assert_eq!((seen[1].messages.as_slice(), seen[1].check), (&["checked".to_owned()][..], true));
}

#[tokio::test]
async fn vcs_flow() {
    let vcs = ScriptedVcs::new([
        Err(vcs::Error::Other("location `/unmounted` matches no preopen".to_owned())),
        Ok(VcsTurn::Revision("sha:main".to_owned())),
    ]);
    let provider = Provider::default().vcs(vcs);
    conformance::vcs_flow(&provider).await;
    provider.vcs.assert_exhausted();

    let seen = provider.vcs.seen();
    assert_eq!(seen.len(), 2);
    assert_eq!((seen[0].op, seen[0].path.as_path()), ("head", Path::new("/unmounted")));
    assert_eq!((seen[1].op, seen[1].path.as_path()), ("resolve", Path::new(".")));
    assert_eq!(seen[1].args, ["main"]);
}
