use std::collections::HashMap;
use std::env;
use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use futures::StreamExt;
use omnia::{GuestId, Router, Runtime, TopicRoutes};
use tracing::{Instrument, debug_span, instrument};

use crate::host::WebSocketView;
use crate::host::generated::DuplexIndices;
use crate::host::resource::{EventProxy, Events};

#[instrument("websocket-server", skip(state))]
pub async fn run<S>(state: &S) -> Result<()>
where
    S: Runtime,
    S::StoreCtx: WebSocketView,
{
    let component = env::var("COMPONENT").unwrap_or_else(|_| "unknown".into());
    tracing::info!("starting websocket server for: {component}");

    // Capability probe: a guest exports the websocket handler exactly when its
    // typed indices resolve. Build the per-guest indices once and the route
    // router that selects among them.
    let mut indices = HashMap::new();
    let mut capable = Vec::new();
    for guest in state.registry().guests() {
        if let Ok(websocket) = DuplexIndices::new(guest.instance_pre()) {
            capable.push(guest.id().clone());
            indices.insert(guest.id().clone(), websocket);
        }
    }
    let router =
        Router::build("websocket", &capable, state.registry().routes().websocket().clone())?;
    if router.is_inert() {
        tracing::info!("no guest exports the websocket handler; websocket trigger inert");
        return Ok(());
    }

    let handler = Handler {
        state: state.clone(),
        component,
        indices: Arc::new(indices),
        router: Arc::new(router),
    };

    // handle events from the websocket clients
    while let Some(event) = handler.events().await?.next().await {
        let handler = handler.clone();

        tokio::spawn(async move {
            tracing::info!(monotonic_counter.event_counter = 1, service = %handler.component);

            if let Err(e) = handler.handle(event.clone()).await {
                tracing::error!(
                    monotonic_counter.processing_errors = 1,
                    service = %handler.component,
                    error = %e,
                );
            }
        });
    }

    Ok(())
}

#[derive(Clone)]
struct Handler<S>
where
    S: Runtime,
    S::StoreCtx: WebSocketView,
{
    state: S,
    component: String,
    indices: Arc<HashMap<GuestId, DuplexIndices>>,
    router: Arc<Router<TopicRoutes>>,
}

impl<S> Handler<S>
where
    S: Runtime,
    S::StoreCtx: WebSocketView,
{
    /// Forward event to the wasm guest.
    async fn handle(&self, event: EventProxy) -> Result<()> {
        // Resolve the guest by the event's route; an event with no route falls
        // into the catch-all (sole exporter). A miss is dropped, not an error.
        let guest_id = event
            .route()
            .map_or_else(|| self.router.catch_all(), |route| self.router.resolve(route));
        let Some(guest_id) = guest_id else {
            tracing::debug!("no route for websocket event; dropping");
            return Ok(());
        };
        let (Some(guest), Some(indices)) =
            (self.state.registry().get(guest_id), self.indices.get(guest_id))
        else {
            tracing::debug!("routed guest is not registered or not websocket-capable");
            return Ok(());
        };

        let mut store_data = self.state.store();
        let event_res = store_data
            .websocket()
            .table
            .push(event)
            .map_err(|e| anyhow!("failed to push event: {e}"))?;

        let mut store = self.state.build_store(store_data);
        let instance = self.state.instantiate(guest.instance_pre(), &mut store).await?;
        let websocket = indices.load(&mut store, &instance)?;

        let run = store
            .run_concurrent(async |store| {
                let guest = websocket.omnia_websocket_handler();
                guest
                    .call_handle(store, event_res)
                    .await
                    .map(|_| ())
                    .map_err(anyhow::Error::from)
                    .context("issue handling event")
            })
            .instrument(debug_span!("websocket-handle"));

        tokio::time::timeout(self.state.options().guest_timeout, run)
            .await
            .context("websocket handler timed out")??
    }

    /// Get events for incoming WebSocket events.
    async fn events(&self) -> Result<Events> {
        let store_data = self.state.store();
        let mut store = self.state.build_store(store_data);

        store
            .run_concurrent(async |store| {
                let client = store.with(|mut store| store.get().websocket().ctx.connect()).await?;
                client.events().await
            })
            .await?
    }
}
