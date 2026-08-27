//! The `create` host binding and its answer gate.

use std::fmt;
use std::sync::Arc;

use anyhow::anyhow;
use futures::{FutureExt as _, future};
use omnia::HasMounts;
use wasmtime::component::{Accessor, FutureReader, StreamReader};

use crate::host::generated::omnia::model::completion::{Host, HostWithStore, Session, ToolResult};
use crate::host::session::{CallsProducer, ReplyTask, ResultsConsumer, SessionClose, ToolSession};
use crate::host::tool_host::DirEntry;
use crate::host::workspace::{self, Workspace};
use crate::host::{
    Error, FutureResult, Request, ToolHost, ToolOutcome, WasiModel, WasiModelCtxView,
};

impl<T> HostWithStore<T> for WasiModel
where
    T: HasMounts,
{
    fn create(
        accessor: &Accessor<T, Self>, request: Request, mut results: StreamReader<ToolResult>,
    ) -> impl Future<Output = Result<Session, Error>> {
        std::future::ready(accessor.with(|mut access| {
            if let Err(error) = request.validate() {
                results.close(&mut access)?;
                return Err(error);
            }

            // get workspace
            let workspace = if let Some(grant) = request.grants.workspace.as_ref() {
                let mounts = access.data_mut().mounts();
                let descriptor = access.get().table.get(&grant.root)?;

                match workspace::resolve(descriptor, &mounts, grant) {
                    Ok(workspace) => Some(workspace),
                    Err(error) => {
                        results.close(&mut access)?;
                        return Err(error.into());
                    }
                }
            } else {
                None
            };

            // call model backend with request and tool host "closure"
            let limits = access.get().ctx.limits();
            let allowed = request.tool_names();
            let format = request.format.clone();

            let (session, calls_rx) = ToolSession::new(limits, allowed);
            let tool_host: Arc<dyn ToolHost> = Arc::new(BoundToolHost {
                session: Arc::clone(&session),
                workspace,
            });
            let answer = access.get().ctx.complete(request, tool_host);

            results.pipe(&mut access, ResultsConsumer::new(Arc::clone(&session)))?;
            let mut calls = StreamReader::new(&mut access, CallsProducer::new(calls_rx))?;

            // extract reply from answer
            let close = SessionClose::new(Arc::clone(&session));
            let reply_task = ReplyTask::spawn(async move {
                let _close = close;
                match answer.await {
                    Ok(answer) => session.take_error().map_or_else(|| answer.project(&format), Err),
                    Err(error) => Err(session.take_error().unwrap_or_else(|| error.into())),
                }
            });

            // map the reply task to a future
            let reply_fut = reply_task.join().map(Ok::<_, wasmtime::Error>);
            let reply = match FutureReader::new(&mut access, reply_fut) {
                Ok(reply) => reply,
                Err(error) => {
                    calls.close(&mut access)?;
                    return Err(error.into());
                }
            };

            Ok(Session { calls, reply })
        }))
    }
}

impl Host for WasiModelCtxView<'_> {
    fn convert_error(&mut self, err: Error) -> wasmtime::Result<Error> {
        Ok(err)
    }
}

// The bound tool host, built fresh per completion from the request's grants
// and the session channels the `create` binding minted.
struct BoundToolHost {
    session: Arc<ToolSession>,
    workspace: Option<Workspace>,
}

// Manual because the session and workspace internals (channels, capability
// handles) carry no useful state to print; the lent path is the identity.
impl fmt::Debug for BoundToolHost {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BoundToolHost")
            .field("workspace", &self.workspace.as_ref().map(Workspace::local_path))
            .finish_non_exhaustive()
    }
}

impl BoundToolHost {
    // Run `op` against the lent workspace, or fail when none was granted.
    fn with_workspace<R: Send + 'static>(
        &self, op: &str, path: String, f: impl FnOnce(&Workspace, String) -> FutureResult<R>,
    ) -> FutureResult<R> {
        match &self.workspace {
            Some(workspace) => f(workspace, path),
            None => future::err(anyhow!("{op}(`{path}`) requires grants.workspace")).boxed(),
        }
    }
}

impl ToolHost for BoundToolHost {
    fn call_tool(&self, name: String, arguments: String) -> FutureResult<ToolOutcome> {
        Arc::clone(&self.session).call(name, arguments)
    }

    fn read(&self, path: String) -> FutureResult<Vec<u8>> {
        self.with_workspace("read", path, Workspace::read)
    }

    fn list(&self, path: String) -> FutureResult<Vec<DirEntry>> {
        self.with_workspace("list", path, Workspace::list)
    }

    fn write(&self, path: String, bytes: Vec<u8>) -> FutureResult<()> {
        self.with_workspace("write", path, move |workspace, path| workspace.write(path, bytes))
    }

    fn local_path(&self) -> Option<&std::path::Path> {
        self.workspace.as_ref().map(Workspace::local_path)
    }
}

// The `create` binding driven on a headless store: no component, linker, or
// guest anywhere — `Store::run_concurrent` mints the `Accessor` the binding
// needs, and the test plays the guest end of the session streams through the
// host stream APIs. In-crate because `HostWithStore` is not exported.
#[cfg(test)]
mod tests {
    use std::pin::Pin;
    use std::task::{Context, Poll};
    use std::time::Duration;

    use omnia::MountRegistry;
    use serde_json::{Value, json};
    use tokio::sync::{mpsc, oneshot};
    use wasmtime::component::{
        Destination, FutureConsumer, Source, StreamConsumer, StreamProducer, StreamResult,
    };
    use wasmtime::{Engine, Store, StoreContextMut};
    use wasmtime_wasi::ResourceTable;

    use super::*;
    use crate::host::generated::omnia::model::completion::ToolCall;
    use crate::host::{
        Answer, Format, Function, Grants, Limits, Message, ModelDefault, Reply, Role, Schema, Tool,
        WasiModelCtx,
    };

    struct Ctx {
        model: Box<dyn WasiModelCtx>,
        table: ResourceTable,
        mounts: Arc<MountRegistry>,
    }

    impl HasMounts for Ctx {
        fn mounts(&self) -> Arc<MountRegistry> {
            Arc::clone(&self.mounts)
        }
    }

    fn model_view(ctx: &mut Ctx) -> WasiModelCtxView<'_> {
        WasiModelCtxView {
            ctx: &mut ctx.model,
            table: &mut ctx.table,
        }
    }

    fn headless(model: impl WasiModelCtx) -> Store<Ctx> {
        Store::new(
            &Engine::default(),
            Ctx {
                model: Box::new(model),
                table: ResourceTable::new(),
                mounts: Arc::new(MountRegistry::default()),
            },
        )
    }

    fn request(tools: Vec<Tool>) -> Request {
        Request {
            model: None,
            system: None,
            messages: vec![Message {
                role: Role::User,
                content: "hi".to_owned(),
            }],
            generation: None,
            format: Format::Text,
            tools,
            grants: Grants { workspace: None },
        }
    }

    fn lookup() -> Tool {
        Tool::Function(Function {
            name: "lookup".to_owned(),
            description: "test lookup".to_owned(),
            parameters: "{}".to_owned(),
        })
    }

    fn verdict_schema() -> Format {
        Format::Schema(Schema {
            name: "verdict".to_owned(),
            schema: json!({
                "type": "object",
                "properties": { "verdict": { "type": "string" } },
                "required": ["verdict"],
            })
            .to_string(),
        })
    }

    // An invalid request must be rejected before the backend runs.
    #[derive(Debug)]
    struct Unreached;

    impl WasiModelCtx for Unreached {
        fn complete(&self, _request: Request, _tools: Arc<dyn ToolHost>) -> FutureResult<Answer> {
            panic!("invalid request reached the backend")
        }
    }

    // Answers every completion with one fixed value, so (unlike the echo) it
    // can satisfy `format::schema`.
    #[derive(Debug)]
    struct Canned(Value);

    impl WasiModelCtx for Canned {
        fn complete(&self, _request: Request, _tools: Arc<dyn ToolHost>) -> FutureResult<Answer> {
            let answer = Answer {
                value: self.0.clone(),
                usage: None,
                transcript: None,
            };
            async move { Ok(answer) }.boxed()
        }
    }

    // A backend that drives the session's tool loop: `calls` sequential
    // invocations of `tool`, answering with the last outcome. Inner tool
    // failures stay model-visible; hard session errors propagate.
    #[derive(Debug)]
    struct ToolDriver {
        tool: &'static str,
        calls: u32,
        limits: Limits,
    }

    impl WasiModelCtx for ToolDriver {
        fn complete(
            &self, _request: Request, tool_host: Arc<dyn ToolHost>,
        ) -> FutureResult<Answer> {
            let tool = self.tool.to_owned();
            let calls = self.calls;
            async move {
                let mut answer = String::new();
                for _ in 0..calls {
                    answer = match tool_host.call_tool(tool.clone(), "{}".to_owned()).await? {
                        Ok(output) => output,
                        Err(failure) => format!("tool failed: {failure}"),
                    };
                }
                Ok(Answer {
                    value: Value::String(answer),
                    usage: None,
                    transcript: None,
                })
            }
            .boxed()
        }

        fn limits(&self) -> Limits {
            self.limits
        }
    }

    // The guest end of the results stream, fed from the drive loop.
    struct ResultsProducer {
        rx: mpsc::Receiver<ToolResult>,
    }

    impl<D> StreamProducer<D> for ResultsProducer {
        type Buffer = Option<ToolResult>;
        type Item = ToolResult;

        fn poll_produce(
            self: Pin<&mut Self>, cx: &mut Context<'_>, _store: StoreContextMut<D>,
            mut destination: Destination<'_, Self::Item, Self::Buffer>, finish: bool,
        ) -> Poll<wasmtime::Result<StreamResult>> {
            match self.get_mut().rx.poll_recv(cx) {
                Poll::Ready(Some(result)) => {
                    destination.set_buffer(Some(result));
                    Poll::Ready(Ok(StreamResult::Completed))
                }
                Poll::Ready(None) => Poll::Ready(Ok(StreamResult::Dropped)),
                Poll::Pending if finish => Poll::Ready(Ok(StreamResult::Cancelled)),
                Poll::Pending => Poll::Pending,
            }
        }
    }

    // The guest end of the calls stream, forwarding to the drive loop.
    struct CallsConsumer {
        tx: mpsc::UnboundedSender<ToolCall>,
    }

    impl<D> StreamConsumer<D> for CallsConsumer {
        type Item = ToolCall;

        fn poll_consume(
            self: Pin<&mut Self>, _cx: &mut Context<'_>, mut store: StoreContextMut<D>,
            mut source: Source<'_, Self::Item>, finish: bool,
        ) -> Poll<wasmtime::Result<StreamResult>> {
            let mut buffer: Vec<ToolCall> = Vec::with_capacity(8);
            source.read(&mut store, &mut buffer)?;
            let took = buffer.len();
            for call in buffer {
                if self.tx.send(call).is_err() {
                    return Poll::Ready(Ok(StreamResult::Dropped));
                }
            }
            if took == 0 && finish {
                return Poll::Ready(Ok(StreamResult::Cancelled));
            }
            Poll::Ready(Ok(StreamResult::Completed))
        }
    }

    // Forwards the piped reply outcome to a oneshot the drive loop awaits.
    struct ReplyConsumer(Option<oneshot::Sender<Result<Reply, Error>>>);

    impl<D> FutureConsumer<D> for ReplyConsumer {
        type Item = Result<Reply, Error>;

        fn poll_consume(
            self: Pin<&mut Self>, _cx: &mut Context<'_>, mut store: StoreContextMut<D>,
            mut source: Source<'_, Self::Item>, _finish: bool,
        ) -> Poll<wasmtime::Result<()>> {
            let mut outcomes: Vec<Result<Reply, Error>> = Vec::with_capacity(1);
            source.read(&mut store, &mut outcomes)?;
            let this = self.get_mut();
            if let (Some(sender), Some(outcome)) = (this.0.take(), outcomes.pop()) {
                drop(sender.send(outcome));
            }
            Poll::Ready(Ok(()))
        }
    }

    // Run one completion through the real `create` binding, playing the
    // guest: every incoming tool call is recorded and answered by `handler`
    // (`None` leaves the call unanswered, e.g. to trip the timeout). Returns
    // the reply outcome and the calls the guest saw.
    async fn drive(
        store: &mut Store<Ctx>, request: Request,
        mut handler: impl FnMut(&ToolCall) -> Option<ToolOutcome>,
    ) -> (Result<Reply, Error>, Vec<ToolCall>) {
        store
            .run_concurrent(async move |accessor| {
                let model = accessor.with_getter::<WasiModel>(model_view);

                let (results_tx, results_rx) = mpsc::channel(8);
                let results = accessor
                    .with(|mut access| {
                        StreamReader::new(&mut access, ResultsProducer { rx: results_rx })
                    })
                    .expect("results stream on a concurrency-enabled store");

                let session =
                    match <WasiModel as HostWithStore<Ctx>>::create(&model, request, results).await
                    {
                        Ok(session) => session,
                        Err(error) => return (Err(error), Vec::new()),
                    };
                let Session { calls, reply } = session;

                let (calls_tx, mut calls_rx) = mpsc::unbounded_channel();
                let (reply_tx, reply_rx) = oneshot::channel();
                accessor
                    .with(|mut access| {
                        calls.pipe(&mut access, CallsConsumer { tx: calls_tx })?;
                        reply.pipe(&mut access, ReplyConsumer(Some(reply_tx)))
                    })
                    .expect("session wires to the test guest");

                // The host always resolves the reply and then closes the
                // calls stream, so joining both cannot hang.
                let mut received = Vec::new();
                let guest = async {
                    while let Some(call) = calls_rx.recv().await {
                        let output = handler(&call);
                        let id = call.id.clone();
                        received.push(call);
                        let Some(output) = output else { continue };
                        if results_tx.send(ToolResult { id, output }).await.is_err() {
                            break;
                        }
                    }
                };
                let ((), outcome) = futures::join!(guest, async {
                    reply_rx.await.expect("host resolves the reply")
                });
                (outcome, received)
            })
            .await
            .expect("headless store run")
    }

    #[tokio::test]
    async fn echo_text() {
        let mut store = headless(ModelDefault);
        let mut two_turns = request(vec![]);
        two_turns.messages.push(Message {
            role: Role::User,
            content: "second".to_owned(),
        });

        let (outcome, calls) = drive(&mut store, two_turns, |_| None).await;

        let reply = outcome.expect("echo answers");
        assert_eq!(reply.answer, "second");
        assert!(reply.usage.is_none());
        assert!(calls.is_empty());
    }

    #[tokio::test]
    async fn echo_json() {
        let mut store = headless(ModelDefault);
        let mut json_format = request(vec![]);
        json_format.format = Format::Json;

        let (outcome, _) = drive(&mut store, json_format, |_| None).await;

        assert_eq!(outcome.expect("echo answers json").answer, r#"{"echo":"hi"}"#);
    }

    #[tokio::test]
    async fn echo_schema() {
        let mut store = headless(ModelDefault);
        let mut schema_format = request(vec![]);
        schema_format.format = verdict_schema();

        let (outcome, _) = drive(&mut store, schema_format, |_| None).await;

        // A backend failure with no session error surfaces as `backend`.
        let error = outcome.expect_err("echo cannot satisfy a schema");
        assert!(
            matches!(error, Error::Backend(detail) if detail.contains("cannot satisfy format::schema"))
        );
    }

    #[tokio::test]
    async fn invalid_request_at_create() {
        let mut store = headless(Unreached);
        let mut empty = request(vec![]);
        empty.messages.clear();

        let (outcome, calls) = drive(&mut store, empty, |_| None).await;

        let error = outcome.expect_err("create refuses an empty request");
        assert!(matches!(error, Error::InvalidRequest(detail) if detail == "empty request"));
        assert!(calls.is_empty());
    }

    #[tokio::test]
    async fn canned_schema_answer() {
        let mut store = headless(Canned(json!({ "verdict": "pass" })));
        let mut schema_format = request(vec![]);
        schema_format.format = verdict_schema();

        let (outcome, _) = drive(&mut store, schema_format, |_| None).await;

        assert_eq!(
            outcome.expect("canned answer satisfies the schema").answer,
            r#"{"verdict":"pass"}"#
        );
    }

    #[tokio::test]
    async fn canned_answer_rejected() {
        let mut store = headless(Canned(json!({ "other": 1 })));
        let mut schema_format = request(vec![]);
        schema_format.format = verdict_schema();

        let (outcome, _) = drive(&mut store, schema_format, |_| None).await;

        let error = outcome.expect_err("projection rejects a non-conforming answer");
        assert!(
            matches!(error, Error::InvalidAnswer(detail) if detail.contains("does not conform to schema `verdict`"))
        );
    }

    #[tokio::test]
    async fn tool_round_trip() {
        let mut store = headless(ToolDriver {
            tool: "lookup",
            calls: 1,
            limits: Limits::default(),
        });
        let (outcome, calls) =
            drive(&mut store, request(vec![lookup()]), |_| Some(Ok("42".to_owned()))).await;

        assert_eq!(outcome.expect("tool loop answers").answer, "42");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "lookup");
        assert_eq!(calls[0].arguments, "{}");
        assert!(!calls[0].id.is_empty());
    }

    #[tokio::test]
    async fn tool_failure_visible_to_model() {
        let mut store = headless(ToolDriver {
            tool: "lookup",
            calls: 1,
            limits: Limits::default(),
        });
        let (outcome, _) =
            drive(&mut store, request(vec![lookup()]), |_| Some(Err("no data".to_owned()))).await;

        // An `Err` outcome is repairable model input, not a session failure.
        assert_eq!(outcome.expect("completion still answers").answer, "tool failed: no data");
    }

    #[tokio::test]
    async fn undeclared_tool() {
        let mut store = headless(ToolDriver {
            tool: "lookup",
            calls: 1,
            limits: Limits::default(),
        });
        let (outcome, calls) =
            drive(&mut store, request(vec![]), |_| Some(Ok("42".to_owned()))).await;

        let error = outcome.expect_err("undeclared tool fails the completion");
        assert!(matches!(error, Error::ToolFailed(detail) if detail.contains("does not declare")));
        assert!(calls.is_empty());
    }

    #[tokio::test]
    async fn budget_exhausted() {
        let limits = Limits {
            max_tool_calls: 1,
            ..Limits::default()
        };
        let mut store = headless(ToolDriver {
            tool: "lookup",
            calls: 2,
            limits,
        });
        let (outcome, calls) =
            drive(&mut store, request(vec![lookup()]), |_| Some(Ok("42".to_owned()))).await;

        let error = outcome.expect_err("second call exceeds the budget");
        assert!(
            matches!(error, Error::BudgetExhausted(detail) if detail.contains("budget of 1 exhausted"))
        );
        assert_eq!(calls.len(), 1);
    }

    #[tokio::test]
    async fn tool_timeout() {
        let limits = Limits {
            tool_timeout: Duration::from_millis(50),
            ..Limits::default()
        };
        let mut store = headless(ToolDriver {
            tool: "lookup",
            calls: 1,
            limits,
        });
        let (outcome, calls) = drive(&mut store, request(vec![lookup()]), |_| None).await;

        let error = outcome.expect_err("unanswered call times out");
        assert!(
            matches!(error, Error::BudgetExhausted(detail) if detail.contains("no result within"))
        );
        assert_eq!(calls.len(), 1);
    }

    #[tokio::test]
    async fn oversize_tool_result() {
        let limits = Limits {
            max_result_bytes: 8,
            ..Limits::default()
        };
        let mut store = headless(ToolDriver {
            tool: "lookup",
            calls: 1,
            limits,
        });
        let (outcome, calls) =
            drive(&mut store, request(vec![lookup()]), |_| Some(Ok("x".repeat(64)))).await;

        let error = outcome.expect_err("oversize result fails the completion");
        assert!(
            matches!(error, Error::ToolFailed(detail) if detail.contains("exceeds the 8-byte cap"))
        );
        assert_eq!(calls.len(), 1);
    }
}
