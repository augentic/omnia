//! A `wasi:http/handler` guest whose response bodies stream on after its
//! handler returns. `/stream/endless` writes a chunk every 10ms and never
//! ends, and `/stream/trap` writes one chunk, then traps.

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

// `thread::sleep` suspends the guest, so the host has sent the head and every
// chunk before the next is made
const TICK: Duration = Duration::from_millis(10);

http::export!(router);

fn router() -> Router {
    Router::new().route("/stream/endless", get(endless)).route("/stream/trap", get(trap))
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
