//! wRPC serve side for host-mediated exports.

use std::collections::HashMap;
use std::pin::pin;
use std::sync::Arc;

use anyhow::{Context as _, Result};
use futures::StreamExt as _;
use wasmtime::component::types;
use wrpc_wasmtime::ServeExt as _;

use super::transport::{InProcServer, InProcess};
use crate::registry::GuestId;
use crate::runtime::Runtime;

/// wRPC host-resource map shape (empty for the resource-free dynamic path).
type HostResources = HashMap<
    Box<str>,
    HashMap<Box<str>, (wasmtime::component::ResourceType, wasmtime::component::ResourceType)>,
>;

/// Wire the serve side of every host-mediated interface.
///
/// Each target guest that exports a linked interface runs a wRPC server whose
/// handlers instantiate the guest *fresh per call* (instance-per-call); the
/// bound transport is then installed so polyfilled imports can reach it.
///
/// Spawns one detached task per served function to drain its invocation stream.
/// A no-op when no guest declares any `link` interface.
///
/// # Errors
///
/// Returns an error if a guest's export cannot be served over the carrier.
pub async fn serve_links<B>(state: &Runtime<B>) -> Result<()>
where
    B: Clone + Send + Sync + 'static,
{
    let registry = state.registry();
    let handle = registry.dispatch();
    if handle.links().is_empty() {
        return Ok(());
    }
    let engine = registry.engine().clone();

    let mut servers: HashMap<GuestId, Arc<InProcServer>> = HashMap::new();
    for guest in registry.guests() {
        let component_ty = guest.component().component_type();
        let mut server: Option<Arc<InProcServer>> = None;

        for (interface, types::ComponentExtern { ty, .. }) in component_ty.exports(&engine) {
            if !handle.links().contains(interface) {
                continue;
            }
            let types::ComponentItem::ComponentInstance(instance_ty) = ty else {
                continue;
            };
            for (func, types::ComponentExtern { ty, .. }) in instance_ty.exports(&engine) {
                let types::ComponentItem::ComponentFunc(func_ty) = ty else {
                    continue;
                };
                let server =
                    Arc::clone(server.get_or_insert_with(|| Arc::new(InProcServer::default())));
                let runtime = state.clone();
                let factory = move || runtime.build_store(runtime.store());
                let stream = server
                    .serve_function(
                        factory,
                        guest.instance_pre().clone(),
                        Arc::<HostResources>::default(),
                        func_ty,
                        interface,
                        func,
                    )
                    .await
                    .with_context(|| {
                        format!("serving `{interface}/{func}` from guest `{}`", guest.id())
                    })?;

                tokio::spawn(async move {
                    let mut stream = pin!(stream);
                    while let Some(invocation) = stream.next().await {
                        match invocation {
                            Ok((_cx, fut)) => {
                                tokio::spawn(async move {
                                    if let Err(error) = fut.await {
                                        tracing::error!(%error, "link serve invocation failed");
                                    }
                                });
                            }
                            Err(error) => tracing::error!(%error, "link serve accept failed"),
                        }
                    }
                });
            }
        }

        if let Some(server) = server {
            servers.insert(guest.id().clone(), server);
        }
    }

    handle.install(InProcess::new(servers));
    Ok(())
}
