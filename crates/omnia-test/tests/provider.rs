//! `omnia_test::guest::Provider` delegation and shared storage.

use omnia_guest::model::{Message, Model, Request, Role};
use omnia_guest::{BlobStore, Config, Identity, Publish, StateStore};
use omnia_test::guest::{FixedIdentity, MapConfig, Provider, Scripted};

fn user(content: &str) -> Request {
    Request::builder()
        .messages(vec![Message {
            role: Role::User,
            content: content.to_owned(),
        }])
        .build()
}

#[tokio::test]
async fn provider_seeds() {
    let provider = Provider::default()
        .config(MapConfig::default().with([("region", "eu")]))
        .identity(FixedIdentity::new("tok"))
        .model(Scripted::answering(["hi"]));
    provider.storage.insert_state("seen", b"1");

    assert_eq!(Config::get(&provider, "region").await.expect("config"), "eu");
    assert_eq!(provider.access_token("svc".into()).await.expect("token"), "tok");
    assert_eq!(StateStore::get(&provider, "seen").await.expect("state"), Some(b"1".to_vec()));
    BlobStore::put(&provider, "c", "o", b"blob").await.expect("blob");
    assert_eq!(
        provider.storage.object("c", "o"),
        Some(b"blob".to_vec()),
        "StateStore and BlobStore share the one storage double"
    );
    assert_eq!(provider.complete(user("q")).await.expect("reply").answer, "hi");
    Publish::send(&provider, "t", &omnia_guest::Message::new(b"m")).await.expect("publish");
    assert_eq!(provider.publish.sent().len(), 1);
    assert!(provider.broadcast.broadcasts().is_empty(), "Publish and Broadcast are separate sinks");
    provider.model.assert_exhausted();
}
