//! Every candidate is rejected: the backend's round budget ends the
//! completion as `exhausted`, the outcome the question answers with.

#![cfg(target_arch = "wasm32")]

use omnia_sdk::model::{Budget, Question};
use test_programs::Verdict;

omnia_sdk::command::export!(scenario);

async fn scenario() {
    let exhausted = Question::<Verdict>::new("verdict")
        .ask("judge this", None, Verdict::passing)
        .await
        .expect("the question runs")
        .expect_err("no candidate passes the check");
    assert_eq!(exhausted.budget, Budget::Responses, "unexpected: {exhausted:?}");

    // the scripted backend's detail is the last correction the check sent
    let detail = &exhausted.detail;
    assert!(detail.contains("## Previous answer (rejected)"), "unexpected: {detail}");
    assert!(detail.contains("verdict must be `pass`"), "unexpected: {detail}");
}
