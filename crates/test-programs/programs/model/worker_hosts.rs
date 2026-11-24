//! Worker grants under a `hosts` grant of `*.npmjs.org` and `api.cursor.sh`.
//! Egress those patterns cover completes. Egress reaching past them is
//! `denied` before the backend runs.

#![cfg(target_arch = "wasm32")]

use omnia_sdk::model::{Request, WorkerGrant};
use test_programs::user;

omnia_sdk::command::export!(scenario);

async fn scenario() {
    let reply = request(&["registry.npmjs.org", "*.cdn.npmjs.org", "api.cursor.sh"])
        .complete()
        .await
        .expect("the completion runs")
        .expect("the model answers");
    assert_eq!(reply.answer, "hi");

    for entry in ["*.cursor.sh", "npmjs.com", "*.org"] {
        let error = request(&[entry]).complete().await.expect_err("the egress is refused");
        assert_eq!(error.code(), "denied", "egress `{entry}`: {error:?}");
        assert!(error.description().contains(entry), "egress `{entry}`: {error:?}");
    }
}

// a worker grant over the lent `.`, so the guest's `hosts` grant is what decides
fn request(egress: &[&str]) -> Request {
    let grant = WorkerGrant {
        egress: egress.iter().map(|entry| (*entry).to_owned()).collect(),
        ..WorkerGrant::default()
    };
    Request::builder().messages(vec![user("hi")]).workspace(".").worker(grant).build()
}
