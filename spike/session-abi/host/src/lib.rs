//! Spike host: implements the session `create` import in wasmtime 47
//! concurrent bindgen mode and exposes a scenario runner for the drop-matrix
//! tests.
//!
//! The scripted "backend" (the test body) talks to the session purely over
//! tokio channels; all store-coupled work happens inside the stream/future
//! producer and consumer callbacks that wasmtime drives from its event loop:
//!
//! - calls stream: host-created via `StreamReader::new` + a `StreamProducer`
//!   that pulls from an mpsc receiver (backend pushes, guest reads).
//! - reply future: host-created via `FutureReader::new` + the blanket
//!   `FutureProducer` impl for plain Rust futures (a oneshot receiver).
//! - results stream: guest-created; host attaches a `StreamConsumer` via
//!   `StreamReader::pipe` that forwards into an unbounded mpsc sender.

use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::task::{Context, Poll};

use wasmtime::component::{
    Accessor, Component, Destination, Linker, Source, StreamConsumer, StreamProducer,
    StreamReader, StreamResult,
};
use wasmtime::error::Context as _;
use wasmtime::{Engine, Store, StoreContextMut};
use wasmtime_wasi::{ResourceTable, WasiCtx, WasiCtxView, WasiView};

pub mod bindings {
    wasmtime::component::bindgen!({
        world: "spike",
        path: "../wit",
        imports: { default: store | trappable },
    });
}

use bindings::spike::session::session::{
    Error as SessionError, HostWithStore, Reply, Session, ToolCall, ToolResult,
};
use wasmtime::component::FutureReader;

pub use bindings::spike::session::session::{
    Error as WireError, Reply as WireReply, ToolCall as WireToolCall,
    ToolResult as WireToolResult,
};

/// Channel ends handed to `create` (via the store data) when the guest opens
/// the session.
pub struct Plumbing {
    calls_rx: tokio::sync::mpsc::Receiver<ToolCall>,
    results_tx: tokio::sync::mpsc::UnboundedSender<ToolResult>,
    reject_results: Arc<AtomicBool>,
    reply_rx: tokio::sync::oneshot::Receiver<Result<Reply, SessionError>>,
    created_tx: tokio::sync::oneshot::Sender<String>,
}

/// Channel ends kept by the scripted backend (the test body).
pub struct Ends {
    /// Write tool-calls; dropping this closes the calls stream.
    pub calls: tokio::sync::mpsc::Sender<ToolCall>,
    /// Tool-results forwarded from the guest's results stream.
    pub results: tokio::sync::mpsc::UnboundedReceiver<ToolResult>,
    /// Resolve the reply future; dropping without sending is the
    /// `reply_dropped` scenario.
    pub reply: tokio::sync::oneshot::Sender<Result<Reply, SessionError>>,
    /// When set, the results consumer reports `StreamResult::Dropped` to the
    /// guest instead of accepting the write (models the host dropping the
    /// results read end).
    pub reject_results: Arc<AtomicBool>,
    /// Resolves with the request string when the guest calls `create`.
    pub created: tokio::sync::oneshot::Receiver<String>,
}

/// Create the paired session plumbing for one scenario run.
pub fn plumbing() -> (Plumbing, Ends) {
    let (calls_tx, calls_rx) = tokio::sync::mpsc::channel(8);
    let (results_tx, results_rx) = tokio::sync::mpsc::unbounded_channel();
    let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
    let (created_tx, created_rx) = tokio::sync::oneshot::channel();
    let reject_results = Arc::new(AtomicBool::new(false));
    (
        Plumbing {
            calls_rx,
            results_tx,
            reject_results: reject_results.clone(),
            reply_rx,
            created_tx,
        },
        Ends {
            calls: calls_tx,
            results: results_rx,
            reply: reply_tx,
            reject_results,
            created: created_rx,
        },
    )
}

/// Store data: WASI for the guest's std imports plus the one-shot session
/// plumbing that `create` consumes.
pub struct Ctx {
    wasi: WasiCtx,
    table: ResourceTable,
    plumbing: Option<Plumbing>,
}

impl Ctx {
    fn new(plumbing: Plumbing) -> Self {
        Self {
            wasi: WasiCtx::builder().inherit_stderr().build(),
            table: ResourceTable::new(),
            plumbing: Some(plumbing),
        }
    }
}

impl WasiView for Ctx {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi,
            table: &mut self.table,
        }
    }
}

impl bindings::spike::session::session::Host for Ctx {}

/// Pulls backend-scripted tool-calls out of an mpsc receiver; a closed
/// channel (backend dropped the sender) closes the stream toward the guest.
struct CallsProducer {
    rx: tokio::sync::mpsc::Receiver<ToolCall>,
}

impl StreamProducer<Ctx> for CallsProducer {
    type Item = ToolCall;
    type Buffer = Option<ToolCall>;

    fn poll_produce(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        _store: StoreContextMut<Ctx>,
        mut destination: Destination<'_, Self::Item, Self::Buffer>,
        finish: bool,
    ) -> Poll<wasmtime::Result<StreamResult>> {
        let this = self.get_mut();
        match this.rx.poll_recv(cx) {
            Poll::Ready(Some(call)) => {
                destination.set_buffer(Some(call));
                Poll::Ready(Ok(StreamResult::Completed))
            }
            Poll::Ready(None) => Poll::Ready(Ok(StreamResult::Dropped)),
            Poll::Pending if finish => Poll::Ready(Ok(StreamResult::Cancelled)),
            Poll::Pending => Poll::Pending,
        }
    }
}

/// Forwards guest tool-results into an unbounded mpsc sender; when
/// `reject` is set it reports `Dropped` instead, so the guest's pending
/// write observes the host abandoning the read end.
struct ResultsConsumer {
    tx: tokio::sync::mpsc::UnboundedSender<ToolResult>,
    reject: Arc<AtomicBool>,
}

impl StreamConsumer<Ctx> for ResultsConsumer {
    type Item = ToolResult;

    fn poll_consume(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        mut store: StoreContextMut<Ctx>,
        mut source: Source<'_, Self::Item>,
        finish: bool,
    ) -> Poll<wasmtime::Result<StreamResult>> {
        let this = self.get_mut();
        if this.reject.load(Ordering::Relaxed) {
            return Poll::Ready(Ok(StreamResult::Dropped));
        }
        let mut buffer: Vec<ToolResult> = Vec::with_capacity(16);
        source.read(&mut store, &mut buffer)?;
        let took = buffer.len();
        for item in buffer {
            // The backend may have finished with the results channel; that is
            // not an error for the guest.
            let _ = this.tx.send(item);
        }
        if took == 0 && finish {
            return Poll::Ready(Ok(StreamResult::Cancelled));
        }
        Poll::Ready(Ok(StreamResult::Completed))
    }
}

/// `HasData` marker implementing the generated host trait.
pub struct SpikeApi;

impl wasmtime::component::HasData for SpikeApi {
    type Data<'a> = &'a mut Ctx;
}

impl HostWithStore<Ctx> for SpikeApi {
    async fn create(
        accessor: &Accessor<Ctx, Self>,
        request: String,
        mut results: StreamReader<ToolResult>,
    ) -> wasmtime::Result<Result<Session, SessionError>> {
        accessor.with(|mut access| {
            let plumbing = access.get().plumbing.take();
            let Some(p) = plumbing else {
                // One session per store in this spike.
                results.close(&mut access)?;
                return Ok(Err(SessionError::Failed(
                    "create called twice: no plumbing left".to_string(),
                )));
            };
            let _ = p.created_tx.send(request);

            let calls = StreamReader::new(&mut access, CallsProducer { rx: p.calls_rx })?;

            let reply_rx = p.reply_rx;
            let reply = FutureReader::new(&mut access, async move {
                reply_rx
                    .await
                    .map_err(|_| wasmtime::Error::msg("reply writer dropped without a value"))
            })?;

            results.pipe(
                &mut access,
                ResultsConsumer {
                    tx: p.results_tx,
                    reject: p.reject_results,
                },
            )?;

            Ok(Ok(Session { calls, reply }))
        })
    }
}

fn guest_wasm_path() -> std::path::PathBuf {
    let mut path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.pop();
    path.push("target/wasm32-wasip2/debug/spike_guest.wasm");
    path
}

fn instance_pre() -> wasmtime::Result<&'static bindings::SpikePre<Ctx>> {
    static PRE: OnceLock<bindings::SpikePre<Ctx>> = OnceLock::new();
    static INIT: Mutex<()> = Mutex::new(());

    if let Some(pre) = PRE.get() {
        return Ok(pre);
    }
    let _guard = INIT.lock().unwrap();
    if let Some(pre) = PRE.get() {
        return Ok(pre);
    }

    let path = guest_wasm_path();
    wasmtime::ensure!(
        path.exists(),
        "guest artifact missing at {path:?} — build it first:\n  \
         cargo build -p spike-guest --target wasm32-wasip2"
    );
    let engine = Engine::default();
    let component = Component::from_file(&engine, &path).context("loading guest component")?;
    let mut linker: Linker<Ctx> = Linker::new(&engine);
    wasmtime_wasi::p2::add_to_linker_async(&mut linker).context("adding WASI to linker")?;
    bindings::Spike::add_to_linker::<Ctx, SpikeApi>(&mut linker, |ctx| ctx)
        .context("adding spike session to linker")?;
    let pre = bindings::SpikePre::new(linker.instantiate_pre(&component)?)?;
    let _ = PRE.set(pre);
    Ok(PRE.get().unwrap())
}

/// Instantiate a fresh store + guest and drive `run(scenario)` to completion.
///
/// Outer `Err` is a trap or runtime failure; the inner result is the guest's
/// own scenario verdict (`;`-joined event log on success).
pub async fn run_scenario(
    scenario: &str,
    plumbing: Plumbing,
) -> wasmtime::Result<Result<String, String>> {
    let pre = instance_pre()?;
    let mut store = Store::new(pre.engine(), Ctx::new(plumbing));
    let spike = pre.instantiate_async(&mut store).await?;
    let scenario = scenario.to_string();
    store
        .run_concurrent(async move |accessor| spike.call_run(accessor, scenario).await)
        .await?
}
