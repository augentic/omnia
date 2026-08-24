//! The end-drop matrix. Every case is timeout-wrapped: a timeout expiring is
//! the failure signal for "left a waiter". Assertions run on both sides —
//! the guest reports its event log through `run`'s return value and the
//! scripted backend reports what it observed on its channel ends.

use std::sync::atomic::Ordering;
use std::time::Duration;

use spike_host::{
    WireError as SessionError, WireReply as Reply, WireToolCall as ToolCall, plumbing,
    run_scenario,
};
use tokio::time::timeout;

const MATRIX_TIMEOUT: Duration = Duration::from_secs(30);

fn call(id: &str, name: &str, arguments: &str) -> ToolCall {
    ToolCall {
        id: id.to_string(),
        name: name.to_string(),
        arguments: arguments.to_string(),
    }
}

fn output(result: &ToolResultLike) -> String {
    match &result.output {
        Ok(v) => format!("ok:{v}"),
        Err(e) => format!("err:{e}"),
    }
}

type ToolResultLike = spike_host::WireToolResult;

#[tokio::test(flavor = "multi_thread")]
async fn round_trip() {
    timeout(MATRIX_TIMEOUT, async {
        let (plumbing, mut ends) = plumbing();
        let backend = tokio::spawn(async move {
            let mut ev = Vec::new();
            let request = ends.created.await.unwrap();
            ev.push(format!("created:{request}"));
            for i in 0..3 {
                let id = format!("c{i}");
                ends.calls
                    .send(call(&id, "echo", &format!("a{i}")))
                    .await
                    .unwrap();
                let r = ends.results.recv().await.unwrap();
                ev.push(format!("result:{}:{}", r.id, output(&r)));
            }
            drop(ends.calls);
            ends.reply
                .send(Ok(Reply {
                    answer: "final-answer".to_string(),
                }))
                .unwrap();
            ev
        });

        let out = run_scenario("round_trip", plumbing).await.unwrap().unwrap();
        assert_eq!(
            out,
            "answered:c0;answered:c1;answered:c2;calls-closed;reply-ok:final-answer"
        );
        assert_eq!(
            backend.await.unwrap(),
            vec![
                "created:round_trip",
                "result:c0:ok:handled:echo:a0",
                "result:c1:ok:handled:echo:a1",
                "result:c2:ok:handled:echo:a2",
            ]
        );
    })
    .await
    .expect("round_trip left a waiter");
}

#[tokio::test(flavor = "multi_thread")]
async fn parallel_calls() {
    timeout(MATRIX_TIMEOUT, async {
        let (plumbing, mut ends) = plumbing();
        let backend = tokio::spawn(async move {
            let mut ev = Vec::new();
            let request = ends.created.await.unwrap();
            ev.push(format!("created:{request}"));
            // Several calls in flight before any result: the guest batches
            // them and answers in reverse order, so the results stream is
            // unordered and must correlate by id.
            for i in 0..3 {
                ends.calls
                    .send(call(&format!("c{i}"), "echo", &format!("a{i}")))
                    .await
                    .unwrap();
            }
            for _ in 0..3 {
                let r = ends.results.recv().await.unwrap();
                ev.push(format!("result:{}", r.id));
            }
            drop(ends.calls);
            ends.reply
                .send(Ok(Reply {
                    answer: "parallel-done".to_string(),
                }))
                .unwrap();
            ev
        });

        let out = run_scenario("parallel_calls", plumbing)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            out,
            "answered:c2;answered:c1;answered:c0;calls-closed;reply-ok:parallel-done"
        );
        assert_eq!(
            backend.await.unwrap(),
            vec![
                "created:parallel_calls",
                "result:c2",
                "result:c1",
                "result:c0",
            ]
        );
    })
    .await
    .expect("parallel_calls left a waiter");
}

#[tokio::test(flavor = "multi_thread")]
async fn guest_drops_session() {
    timeout(MATRIX_TIMEOUT, async {
        let (plumbing, mut ends) = plumbing();
        let backend = tokio::spawn(async move {
            let mut ev = Vec::new();
            let request = ends.created.await.unwrap();
            ev.push(format!("created:{request}"));
            ends.calls.send(call("c0", "echo", "a0")).await.unwrap();
            let r = ends.results.recv().await.unwrap();
            ev.push(format!("result:{}", r.id));
            // The guest drops the calls reader and reply future after the
            // first answer. Both host-side write ends must observe the drop
            // while the instance is still live (it heartbeats on results).
            ends.calls.closed().await;
            ev.push("calls-end-dropped".to_string());
            ends.reply.closed().await;
            ev.push("reply-end-dropped".to_string());
            // Ack via the results consumer so the guest can exit.
            ends.reject_results.store(true, Ordering::Relaxed);
            ev
        });

        let out = run_scenario("guest_drops_session", plumbing)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            out,
            "answered:c0;dropped-session;host-acked-via-results-reject"
        );
        assert_eq!(
            backend.await.unwrap(),
            vec![
                "created:guest_drops_session",
                "result:c0",
                "calls-end-dropped",
                "reply-end-dropped",
            ]
        );
    })
    .await
    .expect("guest_drops_session left a waiter");
}

#[tokio::test(flavor = "multi_thread")]
async fn host_budget_exhausted() {
    timeout(MATRIX_TIMEOUT, async {
        let (plumbing, mut ends) = plumbing();
        let backend = tokio::spawn(async move {
            let mut ev = Vec::new();
            let request = ends.created.await.unwrap();
            ev.push(format!("created:{request}"));
            ends.calls.send(call("c0", "echo", "a0")).await.unwrap();
            let r = ends.results.recv().await.unwrap();
            ev.push(format!("result:{}", r.id));
            // Budget spent: close calls and fail the reply.
            drop(ends.calls);
            ends.reply
                .send(Err(SessionError::BudgetExhausted("spent".to_string())))
                .unwrap();
            ev
        });

        let out = run_scenario("host_budget_exhausted", plumbing)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(out, "answered:c0;calls-closed;reply-err-budget:spent");
        assert_eq!(
            backend.await.unwrap(),
            vec!["created:host_budget_exhausted", "result:c0"]
        );
    })
    .await
    .expect("host_budget_exhausted left a waiter");
}

#[tokio::test(flavor = "multi_thread")]
async fn backend_failure() {
    timeout(MATRIX_TIMEOUT, async {
        let (plumbing, ends) = plumbing();
        let backend = tokio::spawn(async move {
            let mut ev = Vec::new();
            let request = ends.created.await.unwrap();
            ev.push(format!("created:{request}"));
            // Backend fails before consuming any result: the results read
            // end rejects the guest's pending write, calls close, and the
            // reply resolves with the failure.
            ends.reject_results.store(true, Ordering::Relaxed);
            ends.calls.send(call("c0", "echo", "a0")).await.unwrap();
            drop(ends.calls);
            ends.reply
                .send(Err(SessionError::Failed("backend failure".to_string())))
                .unwrap();
            ev
        });

        let out = run_scenario("backend_failure", plumbing)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            out,
            "write-rejected:c0;calls-closed;reply-err-failed:backend failure"
        );
        assert_eq!(backend.await.unwrap(), vec!["created:backend_failure"]);
    })
    .await
    .expect("backend_failure left a waiter");
}

#[tokio::test(flavor = "multi_thread")]
async fn guest_closes_results_early() {
    timeout(MATRIX_TIMEOUT, async {
        let (plumbing, mut ends) = plumbing();
        let backend = tokio::spawn(async move {
            let mut ev = Vec::new();
            let request = ends.created.await.unwrap();
            ev.push(format!("created:{request}"));
            ends.calls.send(call("c0", "echo", "a0")).await.unwrap();
            // The guest drops its results writer without answering: the
            // host's await-result path must see stream end, not hang.
            let end = ends.results.recv().await;
            assert!(end.is_none(), "expected results stream end, got {end:?}");
            ev.push("results-closed".to_string());
            drop(ends.calls);
            ends.reply
                .send(Err(SessionError::Failed(
                    "guest closed results".to_string(),
                )))
                .unwrap();
            ev
        });

        let out = run_scenario("guest_closes_results_early", plumbing)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            out,
            "received:c0;results-writer-dropped;calls-closed;\
             reply-err-failed:guest closed results"
        );
        assert_eq!(
            backend.await.unwrap(),
            vec!["created:guest_closes_results_early", "results-closed"]
        );
    })
    .await
    .expect("guest_closes_results_early left a waiter");
}

/// Not part of the canvas matrix, but pins the sharpest edge found while
/// reading the bindings: the host dropping the reply write end without
/// writing a value is NOT a graceful path.
#[tokio::test(flavor = "multi_thread")]
async fn reply_dropped_without_value() {
    timeout(MATRIX_TIMEOUT, async {
        let (plumbing, ends) = plumbing();
        let backend = tokio::spawn(async move {
            let request = ends.created.await.unwrap();
            drop(ends.calls);
            drop(ends.reply);
            request
        });

        let outcome = run_scenario("reply_dropped", plumbing).await;
        assert_eq!(backend.await.unwrap(), "reply_dropped");
        // Expected: the future producer errors and the guest traps. Pin
        // whatever actually happens so the finding is recorded.
        let err = outcome.expect_err("dropping the reply writer should surface an error");
        let msg = format!("{err:?}");
        assert!(
            msg.contains("reply writer dropped"),
            "unexpected error: {msg}"
        );
    })
    .await
    .expect("reply_dropped_without_value left a waiter");
}
