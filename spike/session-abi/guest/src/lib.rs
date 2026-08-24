//! Spike guest: drives the session shape (stream + future nested in a
//! returned record) through wit-bindgen 0.60 async bindings.
//!
//! Each scenario returns a `;`-joined event log so the host tests can pin
//! exactly what the guest observed.
#![cfg(target_arch = "wasm32")]

wit_bindgen::generate!({
    world: "spike",
    path: "../wit",
});

use spike::session::session::{Error, Session, ToolCall, ToolResult, create};

fn handle(call: &ToolCall) -> ToolResult {
    ToolResult {
        id: call.id.clone(),
        output: Ok(format!("handled:{}:{}", call.name, call.arguments)),
    }
}

fn reply_event(reply: Result<spike::session::session::Reply, Error>) -> String {
    match reply {
        Ok(r) => format!("reply-ok:{}", r.answer),
        Err(Error::Failed(m)) => format!("reply-err-failed:{m}"),
        Err(Error::BudgetExhausted(m)) => format!("reply-err-budget:{m}"),
    }
}

/// The canvas's sugar shape for real: join! over the calls loop and the
/// reply future, answering each call with a closure over locals.
async fn session_loop(scenario: String) -> Result<String, String> {
    let (mut results_tx, results_rx) = wit_stream::new();
    let session = create(scenario, results_rx)
        .await
        .map_err(|e| format!("create failed: {e:?}"))?;
    let Session { mut calls, reply } = session;

    let calls_loop = async {
        let mut events = Vec::new();
        while let Some(call) = calls.next().await {
            match results_tx.write_one(handle(&call)).await {
                None => events.push(format!("answered:{}", call.id)),
                Some(_) => events.push(format!("write-rejected:{}", call.id)),
            }
        }
        events.push("calls-closed".to_string());
        events
    };

    let (mut events, reply_out) =
        futures::join!(calls_loop, std::future::IntoFuture::into_future(reply));
    events.push(reply_event(reply_out));
    Ok(events.join(";"))
}

/// Reads a batch of three calls before answering any, then answers in
/// reverse order: proves ids correlate with the results stream unordered.
async fn parallel_calls() -> Result<String, String> {
    let (mut results_tx, results_rx) = wit_stream::new();
    let session = create("parallel_calls".to_string(), results_rx)
        .await
        .map_err(|e| format!("create failed: {e:?}"))?;
    let Session { mut calls, reply } = session;
    let mut events = Vec::new();

    let mut batch = Vec::new();
    for _ in 0..3 {
        batch.push(calls.next().await.ok_or("calls closed before batch complete")?);
    }
    for call in batch.iter().rev() {
        match results_tx.write_one(handle(call)).await {
            None => events.push(format!("answered:{}", call.id)),
            Some(_) => events.push(format!("write-rejected:{}", call.id)),
        }
    }

    while let Some(call) = calls.next().await {
        events.push(format!("unexpected-call:{}", call.id));
    }
    events.push("calls-closed".to_string());
    events.push(reply_event(reply.await));
    Ok(events.join(";"))
}

/// Answers the first call, then drops the session ends (calls reader +
/// reply future) mid-loop. Heartbeats on the results stream until the host
/// acknowledges by rejecting a write, so the host observes the drops while
/// the instance is still live.
async fn guest_drops_session() -> Result<String, String> {
    let (mut results_tx, results_rx) = wit_stream::new();
    let session = create("guest_drops_session".to_string(), results_rx)
        .await
        .map_err(|e| format!("create failed: {e:?}"))?;
    let Session { mut calls, reply } = session;
    let mut events = Vec::new();

    let call = calls.next().await.ok_or("no first call")?;
    match results_tx.write_one(handle(&call)).await {
        None => events.push(format!("answered:{}", call.id)),
        Some(_) => events.push(format!("write-rejected:{}", call.id)),
    }

    drop(calls);
    drop(reply);
    events.push("dropped-session".to_string());

    loop {
        let heartbeat = ToolResult {
            id: "heartbeat".to_string(),
            output: Ok(String::new()),
        };
        if results_tx.write_one(heartbeat).await.is_some() {
            events.push("host-acked-via-results-reject".to_string());
            break;
        }
    }
    Ok(events.join(";"))
}

/// Receives one call and drops the results writer without answering it.
async fn guest_closes_results_early() -> Result<String, String> {
    let (results_tx, results_rx) = wit_stream::new::<ToolResult>();
    let session = create("guest_closes_results_early".to_string(), results_rx)
        .await
        .map_err(|e| format!("create failed: {e:?}"))?;
    let Session { mut calls, reply } = session;
    let mut events = Vec::new();

    let call = calls.next().await.ok_or("no first call")?;
    events.push(format!("received:{}", call.id));
    drop(results_tx);
    events.push("results-writer-dropped".to_string());

    while let Some(call) = calls.next().await {
        events.push(format!("unexpected-call:{}", call.id));
    }
    events.push("calls-closed".to_string());
    events.push(reply_event(reply.await));
    Ok(events.join(";"))
}

/// Awaits the reply after the host dropped its write end without writing:
/// pins whether that path is graceful or a trap.
async fn reply_dropped() -> Result<String, String> {
    let (_results_tx, results_rx) = wit_stream::new::<ToolResult>();
    let session = create("reply_dropped".to_string(), results_rx)
        .await
        .map_err(|e| format!("create failed: {e:?}"))?;
    let Session { mut calls, reply } = session;
    let mut events = Vec::new();

    while let Some(call) = calls.next().await {
        events.push(format!("unexpected-call:{}", call.id));
    }
    events.push("calls-closed".to_string());
    events.push(reply_event(reply.await));
    Ok(events.join(";"))
}

struct Spike;

impl Guest for Spike {
    async fn run(scenario: String) -> Result<String, String> {
        match scenario.as_str() {
            "round_trip" | "host_budget_exhausted" | "backend_failure" => {
                session_loop(scenario).await
            }
            "parallel_calls" => parallel_calls().await,
            "guest_drops_session" => guest_drops_session().await,
            "guest_closes_results_early" => guest_closes_results_early().await,
            "reply_dropped" => reply_dropped().await,
            other => Err(format!("unknown scenario: {other}")),
        }
    }
}

export!(Spike);
