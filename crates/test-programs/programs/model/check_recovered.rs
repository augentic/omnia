//! A custom correction goes back as the guest renders it, and a candidate
//! that does not deserialize is corrected like any rejection.

#![cfg(target_arch = "wasm32")]

use omnia_sdk::model::Question;
use test_programs::Verdict;

omnia_sdk::api::command::export!(scenario);

async fn scenario() {
    let verdict = Question::<Verdict>::new("verdict")
        .correction(|previous, findings| format!("REJECTED {previous}: {}", findings.join("; ")))
        .ask("judge this", None, Verdict::passing)
        .await
        .expect("the corrected candidate passes the check");
    assert_eq!(verdict.verdict, "pass");

    let verdict = Question::<Verdict>::new("verdict")
        .ask("judge this", None, Verdict::passing)
        .await
        .expect("the candidate after the mismatch passes the check");
    assert_eq!(verdict.verdict, "pass");
}
