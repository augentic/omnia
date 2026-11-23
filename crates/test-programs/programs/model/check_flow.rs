//! The guest's check over a chain of completions, each answered from the
//! scripted backend's next candidates.
//!
//! - A `Question<T>` round trip: the first candidate passes the check and
//!   comes back as a `T`.
//! - A rejected candidate's correction goes back over the session, and the
//!   second candidate is accepted.
//! - `Request::complete_checked` takes a hand-written check, with no
//!   `Question`.
//! - A custom correction goes back as the guest renders it, and a candidate
//!   that does not deserialize is corrected like any rejection.
//! - A question hands the backend's other tool calls to the guest's tools,
//!   and refuses each one when the guest passes none.
//! - Every candidate is rejected: the backend's round budget ends the
//!   completion as `exhausted`, the outcome the question answers with.

#![cfg(target_arch = "wasm32")]

use std::future::ready;

use omnia_sdk::model::{Budget, Candidate, Question, Request, ToolCall, Tools};
use serde::Deserialize;
use test_programs::{Verdict, lookup, user};

omnia_sdk::command::export!(scenario);

async fn scenario() {
    accepted().await;
    corrected().await;
    plain().await;
    recovered().await;
    tools().await;
    exhausted().await;
}

async fn accepted() {
    let verdict = Question::<Verdict>::new("verdict")
        .system("judge")
        .ask("judge this", None, Verdict::passing)
        .await
        .expect("the question runs")
        .expect("the candidate passes the check");
    assert_eq!(
        verdict,
        Verdict {
            verdict: "pass".to_owned(),
            findings: vec![],
        }
    );
}

async fn corrected() {
    let mut candidates = 0;
    let verdict = Question::<Verdict>::new("verdict")
        .ask("judge this", None, |verdict: &Verdict| {
            candidates += 1;
            verdict.passing()
        })
        .await
        .expect("the question runs")
        .expect("the second candidate passes the check");
    assert_eq!(verdict.verdict, "pass");
    assert_eq!(candidates, 2, "both candidates reached the guest's check");
}

// the check accepts only `hi`, correcting anything else
async fn plain() {
    let request = Request::builder().messages(vec![user("hi")]).build();
    let mut candidates = Vec::new();
    let reply = request
        .complete_checked(
            |call: ToolCall| {
                ready(Err::<String, _>(format!("no tool `{}` is declared", call.name)))
            },
            |candidate: Candidate| {
                candidates.push(candidate.answer.clone());
                ready(if candidate.answer == "hi" {
                    Ok(())
                } else {
                    Err(format!("say `hi`, not `{}`", candidate.answer))
                })
            },
        )
        .await
        .expect("the completion runs")
        .expect("a candidate is accepted");
    assert_eq!(reply.answer, "hi");
    assert_eq!(candidates, ["nope", "hi"]);
}

async fn recovered() {
    let verdict = Question::<Verdict>::new("verdict")
        .correction(|previous, findings| format!("REJECTED {previous}: {}", findings.join("; ")))
        .ask("judge this", None, Verdict::passing)
        .await
        .expect("the question runs")
        .expect("the corrected candidate passes the check");
    assert_eq!(verdict.verdict, "pass");

    let verdict = Question::<Verdict>::new("verdict")
        .ask("judge this", None, Verdict::passing)
        .await
        .expect("the question runs")
        .expect("the candidate after the mismatch passes the check");
    assert_eq!(verdict.verdict, "pass");
}

#[derive(Deserialize)]
struct Lookup {
    id: u32,
}

async fn tools() {
    let question = Question::<Verdict>::new("verdict").tools(vec![lookup()]);
    let tools: Tools = Box::new(|call: ToolCall| {
        Box::pin(async move {
            let Lookup { id } = call.arguments()?;
            Ok(format!("looked up {id}"))
        })
    });
    let verdict = question.ask("judge this", Some(tools), Verdict::passing).await;
    let verdict = verdict.expect("the question runs").expect("the handled call leaves a verdict");
    assert_eq!(verdict.verdict, "pass");

    let verdict = question.ask("judge this", None, Verdict::passing).await;
    let verdict = verdict.expect("the question runs").expect("the refused call leaves a verdict");
    assert_eq!(verdict.verdict, "pass");
}

// last, because a script that runs out on a rejection is the backend's
// round budget, and a rejection with turns left moves on to the next one
async fn exhausted() {
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
