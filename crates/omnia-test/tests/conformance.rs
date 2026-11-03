//! Conformance suite for `omnia_test::guest::Provider` (and a scripted `Vcs`
//! double).

#![cfg(not(target_arch = "wasm32"))]

use std::future::{Future, ready};

use bytes::Bytes;
use http::{Method, Response};
use omnia_sdk::sql::{DataType, Field, Row};
use omnia_sdk::vcs::{self, Change, CloneOptions, Entry, Merged, Rule, Vcs};
use omnia_test::conformance;
use omnia_test::guest::{
    FixedIdentity, MapConfig, MatchedHttp, Provider, ScriptedModel, ScriptedTables,
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
    assert_eq!(provider.publish.sent().len(), 1);
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
}

#[tokio::test]
async fn broadcast_send() {
    let provider = Provider::default();
    conformance::broadcast_send(&provider).await;
    assert_eq!(provider.broadcast.broadcasts().len(), 1);
}

#[tokio::test]
async fn model_complete() {
    let provider = Provider::default().model(ScriptedModel::answering(["conformance", "checked"]));
    conformance::model_complete(&provider).await;
    provider.model.assert_exhausted();
}

struct StubVcs;

impl Vcs for StubVcs {
    fn resolve(
        &self, repo: &str, _revision: &str,
    ) -> impl Future<Output = Result<String, vcs::Error>> + Send {
        ready(if repo == "." {
            Ok("sha:main".to_owned())
        } else {
            Err(vcs::Error::Other(format!("location `{repo}` matches no preopen")))
        })
    }

    fn descends(
        &self, _repo: &str, _ancestor: &str, _descendant: &str,
    ) -> impl Future<Output = Result<bool, vcs::Error>> + Send {
        ready(Ok(true))
    }

    fn head(&self, at: &str) -> impl Future<Output = Result<String, vcs::Error>> + Send {
        ready(if at == "." {
            Ok("sha:head".to_owned())
        } else {
            Err(vcs::Error::Other(format!("location `{at}` matches no preopen")))
        })
    }

    fn commit(
        &self, _at: &str, _message: &str,
    ) -> impl Future<Output = Result<Option<String>, vcs::Error>> + Send {
        ready(Ok(None))
    }

    fn merge(
        &self, _at: &str, _revision: &str, _message: &str, _policy: &[Rule],
    ) -> impl Future<Output = Result<Merged, vcs::Error>> + Send {
        ready(Ok(Merged::default()))
    }

    fn log(
        &self, _repo: &str, _revision: &str, _base: &str,
    ) -> impl Future<Output = Result<Vec<Entry>, vcs::Error>> + Send {
        ready(Ok(vec![]))
    }

    fn init(&self, _at: &str) -> impl Future<Output = Result<(), vcs::Error>> + Send {
        ready(Ok(()))
    }

    fn add(
        &self, _repo: &str, _at: &str, _revision: &str,
    ) -> impl Future<Output = Result<(), vcs::Error>> + Send {
        ready(Ok(()))
    }

    fn remove(&self, _at: &str) -> impl Future<Output = Result<(), vcs::Error>> + Send {
        ready(Ok(()))
    }

    fn pending(&self, _at: &str) -> impl Future<Output = Result<Vec<Change>, vcs::Error>> + Send {
        ready(Ok(vec![]))
    }

    fn clone_repo(
        &self, _url: &str, _at: &str, _options: CloneOptions,
    ) -> impl Future<Output = Result<(), vcs::Error>> + Send {
        ready(Ok(()))
    }

    fn fetch(
        &self, _repo: &str, _remote: &str,
    ) -> impl Future<Output = Result<(), vcs::Error>> + Send {
        ready(Ok(()))
    }

    fn label(
        &self, _repo: &str, _name: &str, _revision: &str,
    ) -> impl Future<Output = Result<(), vcs::Error>> + Send {
        ready(Ok(()))
    }

    fn labelled(
        &self, _repo: &str, _name: &str,
    ) -> impl Future<Output = Result<String, vcs::Error>> + Send {
        ready(Ok(String::new()))
    }

    fn fetched(
        &self, _repo: &str, _remote: &str, _name: &str,
    ) -> impl Future<Output = Result<String, vcs::Error>> + Send {
        ready(Ok(String::new()))
    }

    fn push(
        &self, _repo: &str, _remote: &str, _label: &str,
    ) -> impl Future<Output = Result<(), vcs::Error>> + Send {
        ready(Ok(()))
    }
}

#[tokio::test]
async fn vcs_flow() {
    conformance::vcs_flow(&StubVcs).await;
}
