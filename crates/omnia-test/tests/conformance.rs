//! Conformance suite for the stateful doubles on `omnia_test::guest::Provider`.

#![cfg(not(target_arch = "wasm32"))]

use bytes::Bytes;
use http::{HeaderValue, Method, Response};
use omnia_test::conformance;
use omnia_test::guest::{
    Broadcasted, FixedIdentity, MapConfig, MatchedHttp, Provider, ScriptedModel,
};

omnia_test::conformance!(
    state, docstore, blobstore, publish, config, identity, vault, http, broadcast, model
);

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
