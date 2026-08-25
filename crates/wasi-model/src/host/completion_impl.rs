//! The `create` host binding.
//!
//! Implements the generated `completion` host trait on [`WasiModel`]. It is
//! the host validation gate: it validates the request, mints the session
//! channels (calls stream, reply future) around the backend, pipes the
//! guest's results stream into them, and re-validates the final answer
//! before the reply future resolves. A backend that runs its own repair loop
//! (genai) consumes validation failures internally and only returns once it
//! passes; the host re-validates here.
//!
//! The reply pipeline runs eagerly as a spawned task, since a session guest
//! reads `calls` before awaiting the reply (see `session::ReplyTask`).
//! Cancellation stays structural: the guest dropping its reply future drops
//! the producer awaiting the task, which aborts it — no in-flight backend
//! work outlives the session. The reply future is always resolved with a
//! value; budget and deadline failures are typed `error` values, never a
//! dropped writer (dropping it would trap the guest).

use std::sync::Arc;

use anyhow::anyhow;
use futures::FutureExt as _;
use omnia::HasMounts;
use wasmtime::component::{Accessor, FutureReader, StreamReader};

use crate::host::generated::omnia::model::completion::{
    Host, HostWithStore, Session, Tool, ToolResult,
};
use crate::host::resource::DirEntry;
use crate::host::session::{CallsProducer, ReplyTask, ResultsConsumer, SessionClose, SessionState};
use crate::host::workspace::{self, Workspace};
use crate::host::{Error, Format, FutureResult, Request, ToolHost, WasiModel, WasiModelCtxView};

impl<T> HostWithStore<T> for WasiModel
where
    T: HasMounts,
{
    // The generated trait method is async; opening the session is entirely
    // synchronous store work (the backend runs behind the reply future).
    #[expect(clippy::unused_async_trait_impl)]
    async fn create(
        accessor: &Accessor<T, Self>, mut request: Request, mut results: StreamReader<ToolResult>,
    ) -> Result<Session, Error> {
        // The lent `borrow<descriptor>` cannot survive the backend await, so
        // the host takes it out here to resolve the workspace for `ToolHost`.
        let lent = request.grants.workspace.take();

        if let Err(error) = validate(&request) {
            // A stream not returned to the guest must be disposed explicitly.
            accessor.with(|mut access| results.close(&mut access))?;
            return Err(error);
        }

        let format = request.format.clone();
        // The declared function tools are the only names `call-tool` accepts.
        let allowed: Vec<String> = request
            .tools
            .iter()
            .filter_map(|tool| match tool {
                Tool::Function(function) => Some(function.name.clone()),
                Tool::Mcp(_) => None,
            })
            .collect();

        accessor.with(|mut access| {
            let mounts = access.data_mut().mounts();
            let resolved = {
                let view = access.get();
                workspace::resolve(view.table, &mounts, lent.as_ref())
            };
            let workspace = match resolved {
                Ok(workspace) => workspace,
                Err(error) => {
                    results.close(&mut access)?;
                    return Err(error.into());
                }
            };

            let limits = access.get().ctx.limits();
            let (session, calls_rx) = SessionState::new(limits, allowed);
            let tool_host: Arc<dyn ToolHost> = Arc::new(BoundToolHost {
                session: Arc::clone(&session),
                workspace,
            });
            let backend = access.get().ctx.complete(request, tool_host);

            results.pipe(&mut access, ResultsConsumer::new(Arc::clone(&session)))?;
            let mut calls = StreamReader::new(&mut access, CallsProducer::new(calls_rx))?;

            // The reply pipeline — the backend future piped through the
            // answer gate — runs eagerly as a spawned task (see [`ReplyTask`]
            // for why). It always yields a value; a typed failure recorded by
            // host enforcement wins over whatever the backend then returned.
            // The guard ends the session — completion or cancellation alike —
            // so the guest's calls loop always terminates.
            let close = SessionClose::new(Arc::clone(&session));
            let task = ReplyTask::spawn(async move {
                let _close = close;
                match backend.await {
                    Ok(answer) => {
                        session.take_failure().map_or_else(|| answer.project(&format), Err)
                    }
                    Err(error) => Err(session.take_failure().unwrap_or_else(|| error.into())),
                }
            });
            let reply_future = async move { Ok::<_, wasmtime::Error>(task.join().await) };
            let reply = match FutureReader::new(&mut access, reply_future) {
                Ok(reply) => reply,
                Err(error) => {
                    calls.close(&mut access)?;
                    return Err(error.into());
                }
            };

            Ok(Session { calls, reply })
        })
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
    session: Arc<SessionState>,
    workspace: Option<Workspace>,
}

impl ToolHost for BoundToolHost {
    fn call_tool(&self, name: String, arguments: String) -> FutureResult<Result<String, String>> {
        Arc::clone(&self.session).call(name, arguments)
    }

    fn read(&self, path: String) -> FutureResult<Vec<u8>> {
        let Some(workspace) = self.workspace.as_ref() else {
            return async move { Err(anyhow!("read(`{path}`) requires grants.workspace")) }.boxed();
        };
        workspace.read(path)
    }

    fn list(&self, path: String) -> FutureResult<Vec<DirEntry>> {
        let Some(workspace) = self.workspace.as_ref() else {
            return async move { Err(anyhow!("list(`{path}`) requires grants.workspace")) }.boxed();
        };
        workspace.list(path)
    }

    fn write(&self, path: String, bytes: Vec<u8>) -> FutureResult<()> {
        let Some(workspace) = self.workspace.as_ref() else {
            return async move { Err(anyhow!("write(`{path}`) requires grants.workspace")) }
                .boxed();
        };
        workspace.write(path, bytes)
    }

    fn local_path(&self) -> Option<&std::path::Path> {
        self.workspace.as_ref().map(Workspace::local_path)
    }
}

const TOOL_NAMES: &[&str] = &["read", "list", "write"];

fn validate(request: &Request) -> Result<(), Error> {
    for tool in &request.tools {
        let Tool::Function(function) = tool else {
            continue;
        };
        if TOOL_NAMES.contains(&function.name.as_str()) {
            return Err(Error::InvalidRequest(format!("reserved tool name: {}", function.name)));
        }
        if serde_json::from_str::<serde_json::Value>(&function.parameters).is_err() {
            return Err(Error::InvalidRequest(format!(
                "function tool `{}` parameters is not valid JSON",
                function.name
            )));
        }
    }

    if request.messages.iter().all(|m| m.content.trim().is_empty()) {
        return Err(Error::InvalidRequest("empty request".to_owned()));
    }

    if let Format::Schema(spec) = &request.format {
        let schema: serde_json::Value = serde_json::from_str(&spec.schema)
            .map_err(|e| Error::InvalidRequest(format!("format schema is not valid JSON: {e}")))?;
        jsonschema::validator_for(&schema).map_err(|e| {
            Error::InvalidRequest(format!("format schema is not a valid JSON Schema: {e}"))
        })?;
    }

    Ok(())
}


#[cfg(test)]
mod tests {
    use super::validate;
    use crate::host::Error;
    use crate::host::generated::omnia::model::completion::{
        Format, Function, Grants, Message, Request, Role, Schema, Tool,
    };

    #[test]
    fn reserved_tool_name() {
        let mut request = into_request(vec![message(Role::User, "hi")]);
        request.tools.push(Tool::Function(Function {
            name: "read".to_owned(),
            description: "shadow a host-injected tool".to_owned(),
            parameters: "{}".to_owned(),
        }));
        let err = validate(&request).unwrap_err();
        assert!(matches!(err, Error::InvalidRequest(m) if m.contains("reserved tool name")));
    }

    #[test]
    fn empty_request() {
        let err = validate(&into_request(vec![])).unwrap_err();
        assert!(matches!(err, Error::InvalidRequest(m) if m == "empty request"));

        // messages present but all blank is still empty.
        let err = validate(&into_request(vec![message(Role::User, "   ")])).unwrap_err();
        assert!(matches!(err, Error::InvalidRequest(m) if m == "empty request"));
    }

    // `resolve` is no longer host-injected: a guest may declare it as an
    // ordinary function tool.
    #[test]
    fn resolve_is_valid() {
        let mut request = into_request(vec![message(Role::User, "hi")]);
        request.tools.push(Tool::Function(Function {
            name: "resolve".to_owned(),
            description: "look up a reference".to_owned(),
            parameters: "{\"type\":\"object\"}".to_owned(),
        }));
        validate(&request).unwrap();
    }

    #[test]
    fn invalid_params() {
        let mut request = into_request(vec![message(Role::User, "hi")]);
        request.tools.push(Tool::Function(Function {
            name: "lookup".to_owned(),
            description: "look something up".to_owned(),
            parameters: "not json".to_owned(),
        }));
        let err = validate(&request).unwrap_err();
        assert!(matches!(err, Error::InvalidRequest(m) if m.contains("`lookup`")));
    }

    #[test]
    fn invalid_schema() {
        let mut request = into_request(vec![message(Role::User, "hi")]);
        request.format = Format::Schema(Schema {
            name: "verdict".to_owned(),
            schema: "not json".to_owned(),
        });
        let err = validate(&request).unwrap_err();
        assert!(matches!(err, Error::InvalidRequest(m) if m.contains("not valid JSON")));

        let mut request = into_request(vec![message(Role::User, "hi")]);
        request.format = Format::Schema(Schema {
            name: "verdict".to_owned(),
            schema: "{\"type\":\"nonsense\"}".to_owned(),
        });
        let err = validate(&request).unwrap_err();
        assert!(matches!(err, Error::InvalidRequest(m) if m.contains("valid JSON Schema")));
    }

    fn into_request(messages: Vec<Message>) -> Request {
        Request {
            model: None,
            system: None,
            messages,
            generation: None,
            format: Format::Json,
            tools: vec![],
            grants: Grants { workspace: None },
        }
    }

    fn message(role: Role, content: &str) -> Message {
        Message {
            role,
            content: content.to_owned(),
        }
    }
}
