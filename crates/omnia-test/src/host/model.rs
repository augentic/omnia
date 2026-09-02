//! A FIFO-scripted `WasiModelCtx` recording every request.

use std::sync::{Arc, Mutex};

use futures::FutureExt as _;
use omnia_wasi_model::{Answer, Format, FutureResult, Request, Tool, ToolHost, WasiModelCtx};
use serde_json::Value;

use crate::{Exchange, Script, Seen, SeenFormat};

/// One scripted turn: the tool calls the backend drives through the
/// session, then the answer.
#[derive(Clone, Debug)]
pub struct Turn {
    /// `(tool, arguments)` pairs driven through the session's `ToolHost`
    /// before the answer returns.
    pub calls: Vec<(String, String)>,
    /// The answer value the host projects through its format gate.
    pub answer: Value,
}

/// A FIFO model backend recording every request and tool exchange.
///
/// The host-side counterpart of the guest `Scripted` double: answers are
/// JSON values the `omnia:model` host projects to the guest through its
/// format gate, and a turn may drive declared tools through the session
/// before answering. A call past the script fails the completion.
///
/// ```no_run
/// use omnia_test::host::{Backends, Deployment, ScriptedModel};
/// use omnia_wasi_model::WasiModel;
/// use serde_json::json;
///
/// # async fn example(guest: &'static str) -> anyhow::Result<()> {
/// let model = ScriptedModel::answering([json!("42")]).calling(0, [("lookup", "{}")]);
/// let backends = Backends::defaults().await.model(model);
/// Deployment::new().guest("agent", guest).run_host::<WasiModel, _>(backends.clone()).await?;
/// assert_eq!(backends.model.exchanges()[0].tool, "lookup");
/// backends.model.assert_exhausted();
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Debug)]
pub struct ScriptedModel {
    script: Script<Seen, Turn>,
    exchanges: Arc<Mutex<Vec<Exchange>>>,
}

impl Default for ScriptedModel {
    fn default() -> Self {
        Self::answering([])
    }
}

impl ScriptedModel {
    /// A script of ordered answers.
    pub fn answering(answers: impl IntoIterator<Item = Value>) -> Self {
        Self {
            script: Script::new(answers.into_iter().map(|answer| Turn {
                calls: Vec::new(),
                answer,
            })),
            exchanges: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Attaches `(tool, arguments)` calls to the turn at `index`; the
    /// backend drives them through the session before that turn answers.
    ///
    /// # Panics
    ///
    /// Panics when no turn is scripted at `index`.
    #[must_use]
    #[track_caller]
    pub fn calling<T: Into<String>, A: Into<String>>(
        self, index: usize, calls: impl IntoIterator<Item = (T, A)>,
    ) -> Self {
        let calls: Vec<(String, String)> =
            calls.into_iter().map(|(tool, arguments)| (tool.into(), arguments.into())).collect();
        Self {
            script: self.script.edit(index, |turn| turn.calls.extend(calls)),
            exchanges: self.exchanges,
        }
    }

    /// Answers every completion past the scripted turns with `answer`
    /// instead of failing.
    ///
    /// # Panics
    ///
    /// Panics if a fallback was already set.
    #[must_use]
    pub fn then(self, answer: impl Fn() -> Value + Send + Sync + 'static) -> Self {
        Self {
            script: self.script.then(move || Turn {
                calls: Vec::new(),
                answer: answer(),
            }),
            exchanges: self.exchanges,
        }
    }

    /// Every request in call order.
    #[must_use]
    pub fn seen(&self) -> Vec<Seen> {
        self.script.seen()
    }

    /// Every driven tool exchange in call order.
    ///
    /// # Panics
    ///
    /// Panics if the exchange lock is poisoned.
    #[must_use]
    pub fn exchanges(&self) -> Vec<Exchange> {
        self.exchanges.lock().expect("exchanges lock").clone()
    }

    /// Asserts that every scripted answer was consumed.
    ///
    /// # Panics
    ///
    /// Panics naming the number of unconsumed turns.
    #[track_caller]
    pub fn assert_exhausted(&self) {
        self.script.assert_exhausted();
    }
}

impl WasiModelCtx for ScriptedModel {
    fn complete(&self, request: Request, tool_host: Arc<dyn ToolHost>) -> FutureResult<Answer> {
        // A guest that completes past the script must see a backend failure,
        // not a host panic inside the wasmtime call.
        let Some(turn) = self.script.try_next(Seen::from(&request)) else {
            let consumed = self.script.seen().len();
            return async move {
                Err(anyhow::anyhow!(
                    "model script exhausted: {} turn(s) consumed, none scripted for request \
                     #{consumed}",
                    consumed - 1
                ))
            }
            .boxed();
        };
        let exchanges = Arc::clone(&self.exchanges);
        async move {
            for (tool, arguments) in turn.calls {
                let outcome = tool_host.call_tool(tool.clone(), arguments.clone()).await?;
                exchanges.lock().expect("exchanges lock").push(Exchange {
                    tool,
                    arguments,
                    outcome,
                });
            }
            Ok(turn.answer.into())
        }
        .boxed()
    }
}

impl From<&Request> for Seen {
    fn from(request: &Request) -> Self {
        Self {
            system: request.system.clone(),
            messages: request.messages.iter().map(|message| message.content.clone()).collect(),
            format: match &request.format {
                Format::Text => SeenFormat::Text,
                Format::Json => SeenFormat::Json,
                Format::Schema(schema) => SeenFormat::Schema {
                    name: schema.name.clone(),
                    schema: schema.schema.clone(),
                },
            },
            tools: request
                .tools
                .iter()
                .map(|tool| match tool {
                    Tool::Function(function) => function.name.clone(),
                    Tool::Mcp(mcp) => mcp.name.clone(),
                })
                .collect(),
            // The descriptor lend cannot cross into a plain record; the
            // subpath beneath the lent root is what the guest chose.
            workspace: request.grants.workspace.as_ref().map(|grant| grant.subpath.clone()),
        }
    }
}
