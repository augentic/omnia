//! A `wasi:http/handler` guest whose handler stalls, or whose response bodies
//! stream on after its handler returns. `/stream/stall` never answers,
//! `/stream/slow` answers `slow` after a pause of 300ms, `/stream/endless`
//! writes a chunk every 10ms and never ends, and `/stream/trap` writes one
//! chunk, then traps.

#![cfg(target_arch = "wasm32")]

use std::convert::Infallible;
use std::task::Poll;
use std::thread;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::routing::get;
use bytes::Bytes;
use futures::{StreamExt as _, stream};
use omnia_sdk::http;

// `thread::sleep` suspends the guest, so the host can abort it between sleeps,
// and has sent the head and every chunk before the next is made
const TICK: Duration = Duration::from_millis(10);

// long enough for the host to stop while the guest waits
const PAUSE: Duration = Duration::from_millis(300);

http::export!(router);

fn router() -> Router {
    Router::new()
        .route("/stream/stall", get(stall))
        .route("/stream/slow", get(slow))
        .route("/stream/endless", get(endless))
        .route("/stream/trap", get(trap))
}

async fn stall() {
    loop {
        thread::sleep(TICK);
    }
}

async fn slow() -> &'static str {
    thread::sleep(PAUSE);
    "slow"
}

async fn endless() -> Body {
    Body::from_stream(stream::repeat_with(|| {
        thread::sleep(TICK);
        Ok::<_, Infallible>(Bytes::from_static(b"tick\n"))
    }))
}

async fn trap() -> Body {
    let partial = stream::iter([Ok::<_, Infallible>(Bytes::from_static(b"partial"))]);
    let trap = stream::poll_fn(|_| -> Poll<Option<Result<Bytes, Infallible>>> {
        thread::sleep(TICK);
        panic!("trapping mid-body");
    });
    Body::from_stream(partial.chain(trap))
}
