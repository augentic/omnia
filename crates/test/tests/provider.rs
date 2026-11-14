//! `omnia_test::guest::Provider` delegation and shared storage.

#![cfg(not(target_arch = "wasm32"))]

use omnia_sdk::messaging::Message as Published;
use omnia_sdk::model::{Message, Model, Request, Role};
use omnia_sdk::{BlobStore, Config, IdentityExt as _, Publisher, SecretStore, StateStore};
use omnia_test::guest::{FixedIdentity, MapConfig, Memory, Provider, ScriptedModel};

fn user(content: &str) -> Request {
    Request::builder()
        .messages(vec![Message {
            role: Role::User,
            content: content.to_owned(),
        }])
        .build()
}

#[tokio::test]
async fn seeded_doubles() {
    let provider = Provider::default()
        .config(MapConfig::default().with([("region", "eu")]))
        .identity(FixedIdentity::new("tok"))
        .model(ScriptedModel::answering(["hi"]));
    provider.storage.insert_state("seen", b"1");

    assert_eq!(Config::get(&provider, "region").await.expect("config").as_deref(), Some("eu"));
    assert_eq!(provider.access_token().await.expect("token"), "tok");
    assert_eq!(
        StateStore::get(&provider, "seen").await.expect("state").as_deref(),
        Some(&b"1"[..])
    );
    BlobStore::create_container(&provider, "c").await.expect("container");
    BlobStore::put(&provider, "c", "o", b"blob").await.expect("blob");
    SecretStore::set(&provider, "key", b"secret").await.expect("secret");
    assert_eq!(
        (provider.storage.object("c", "o"), provider.storage.secret("key")),
        (Some(b"blob".to_vec()), Some(b"secret".to_vec())),
        "StateStore, BlobStore, and SecretStore share the one storage double"
    );
    assert_eq!(provider.complete(user("q")).await.expect("reply").answer, "hi");
    Publisher::publish(&provider, "t", &Published::new(b"m")).await.expect("publish");
    assert_eq!(provider.publisher.sent().len(), 1);
    assert!(
        provider.websockets.broadcasts().is_empty(),
        "Publisher and WebSockets are separate sinks"
    );
    provider.model.assert_exhausted();
}

// the derived impls forward `named` to the field's double
#[tokio::test]
async fn named_storage() {
    let provider = Provider::default().storage(Memory::default().bucket("cache"));

    let cache = StateStore::named(&provider, "cache");
    let vault = SecretStore::named(&provider, "vault");
    StateStore::set(&cache, "k", b"v", None).await.expect("state");
    SecretStore::set(&vault, "k", b"s").await.expect("secret");

    assert_eq!(provider.storage.named("cache").state("k"), Some(b"v".to_vec()));
    assert_eq!(provider.storage.named("vault").secret("k"), Some(b"s".to_vec()));
    assert_eq!(provider.storage.state("k"), None);
}
