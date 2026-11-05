//! A `Question<T>` round trip: the first candidate passes the guest's check
//! and comes back as a `T`.

#![cfg(target_arch = "wasm32")]

use omnia_sdk::WasiProvider;
use omnia_sdk::model::Question;
use test_programs::Verdict;

omnia_sdk::export!(command: scenario);

async fn scenario() {
    let verdict = Question::<Verdict>::new("verdict")
        .system("judge")
        .ask(&WasiProvider, "judge this", None, Verdict::passing)
        .await
        .expect("the candidate passes the check");
    assert_eq!(
        verdict,
        Verdict {
            verdict: "pass".to_owned(),
            findings: vec![],
        }
    );
}
