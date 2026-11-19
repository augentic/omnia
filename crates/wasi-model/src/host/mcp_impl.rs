//! The optional `mcp` host binding, which offers MCP servers' tools beside
//! the request's own.

use omnia_core::{HostView, StoreCtx};
use wasmtime::component::{Accessor, Resource, StreamReader};

use crate::host::completion_impl::ModelHandle;
use crate::host::generated::omnia::model::completion::{Request, Session};
use crate::host::generated::omnia::model::mcp::{Host, HostWithStore, Server};
use crate::host::generated::omnia::model::types::CallResult;
use crate::host::{Error, ModelClient, WasiModel};

impl<B: Send + 'static> HostWithStore<StoreCtx<B>> for WasiModel {
    fn complete(
        accessor: &Accessor<StoreCtx<B>, Self>, model: Resource<ModelHandle>, request: Request,
        servers: Vec<Server>, results: StreamReader<CallResult>,
    ) -> impl Future<Output = Result<Session, Error>> {
        std::future::ready(
            accessor.with(|mut access| Self::begin(&mut access, &model, request, servers, results)),
        )
    }
}

impl Host for HostView<'_, dyn ModelClient> {}
