//! End-to-end tests for `omnia:websocket`: every scenario runs a real guest
//! component from `crates/test-programs` through the omnia runtime against an
//! inline recording backend. The `handler` export is driven in-process
//! through [`WebSocketHandler`], bypassing the socket. The guest asserts what
//! it observes across the boundary (and traps on failure); the host side
//! asserts what the guest sent back through the client.

#![cfg(not(target_arch = "wasm32"))]

use std::sync::{Arc, Mutex};

use futures::FutureExt as _;
use omnia::{FutureResult, Provides};
use omnia_test::host::Deployment;
use omnia_wasi_otel::{OtelDefault, WasiOtel, WasiOtelCtx};
use omnia_wasi_websocket::{
    Client, Event, Events, WasiWebSocket, WasiWebSocketCtx, WebSocketHandler,
};

// Every guest program in `crates/test-programs` must have a matching test
// here; a new program without one fails to compile.
test_programs::foreach_websocket!();

/// The store's backend bundle: the recording websocket backend under test
/// beside the no-op telemetry sink the guest imports.
#[derive(Clone, Debug)]
struct Backends {
    websocket: Recording,
    otel: OtelDefault,
}

impl Provides<WasiWebSocket> for Backends {
    fn borrow(&mut self) -> &mut dyn WasiWebSocketCtx {
        &mut self.websocket
    }
}

impl Provides<WasiOtel> for Backends {
    fn borrow(&mut self) -> &mut dyn WasiOtelCtx {
        &mut self.otel
    }
}

/// One `send` as the client saw it: the event and its socket filter.
type Sent = (Event, Option<Vec<String>>);

/// Records every event the guest sends, for host-side assertions. The
/// default backend fans out to zero peers, so a send through it is
/// unobservable; this stands in as the scenario backend.
#[derive(Clone, Debug, Default)]
struct Recording {
    sent: Arc<Mutex<Vec<Sent>>>,
}

impl WasiWebSocketCtx for Recording {
    fn connect(&self) -> FutureResult<Arc<dyn Client>> {
        let client = self.clone();
        async move { Ok(Arc::new(client) as Arc<dyn Client>) }.boxed()
    }
}

impl Client for Recording {
    fn events(&self) -> FutureResult<Events> {
        // Events are handed to the handler directly; nothing arrives here.
        async { Ok(Box::pin(futures::stream::empty()) as Events) }.boxed()
    }

    fn send(&self, event: Event, sockets: Option<Vec<String>>) -> FutureResult<()> {
        self.sent.lock().expect("sent lock").push((event, sockets));
        async { Ok(()) }.boxed()
    }
}

#[tokio::test]
async fn websocket_handler_echo() {
    let recording = Recording::default();
    // The handler guest exports no `wasi:cli/run`: boot without driving it
    // and dispatch events through the trigger's in-process handler.
    let runtime = Deployment::new()
        .guest("guest", test_programs::WEBSOCKET_HANDLER_ECHO)
        .boot(
            Backends {
                websocket: recording.clone(),
                otel: OtelDefault,
            },
            |deployment| {
                deployment.host::<WasiWebSocket, Backends>()?;
                deployment.host::<WasiOtel, Backends>()?;
                Ok(())
            },
        )
        .await
        .expect("runtime boots");
    let handler = WebSocketHandler::new(&runtime)
        .expect("websocket routes consistent")
        .expect("the guest exports the websocket handler");

    // An event without a route dispatches through the sole-exporter
    // catch-all; the guest asserts its frame and origin, then replies.
    let mut event = Event::new(b"ping".to_vec());
    event.socket_addr = Some("peer-1".to_owned());
    handler.handle(event).await.expect("handled");

    // Wire fidelity: exactly one reply reached the client, unfiltered, and a
    // guest-built event names no origin.
    let sent = recording.sent.lock().expect("sent lock").clone();
    assert_eq!(sent.len(), 1);
    let (reply, sockets) = &sent[0];
    assert_eq!(reply.data, b"pong");
    assert!(reply.socket_addr.is_none());
    assert!(sockets.is_none(), "an unfiltered send names no sockets");

    runtime.shutdown();
}
