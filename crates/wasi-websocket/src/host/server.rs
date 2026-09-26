use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use futures::StreamExt;
use omnia_core::{PatternRoutes, Runtime, StoreCtx, StoreView, TriggerRouter};
use tracing::{Instrument, debug_span, instrument};

use crate::host::WasiWebSocket;
use crate::host::generated::DuplexIndices;
use crate::host::resource::{Event, Events};

#[instrument("websocket-server", skip(state))]
pub async fn run<B>(state: &Runtime<B>) -> Result<()>
where
    B: Clone + Send + Sync + 'static,
    StoreCtx<B>: StoreView<WasiWebSocket>,
{
    tracing::info!("starting websocket server for: {}", state.name());

    let Some(handler) = WebSocketHandler::new(state)? else {
        tracing::info!("no guest exports the websocket handler; websocket trigger inert");
        return Ok(());
    };

    // Subscribe once: a fresh subscription per iteration would drop events
    // published between polls (broadcast receivers only see what arrives
    // after they subscribe).
    let mut events = handler.events().await?;

    while let Some(event) = events.next().await {
        let handler = handler.clone();

        tokio::spawn(async move {
            tracing::info!(monotonic_counter.event_counter = 1, service = %handler.component);

            if let Err(e) = handler.handle(event).await {
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

/// In-process `omnia:websocket/handler` dispatcher: routes an event to a
/// guest, instantiates it, and invokes the export.
///
/// The server loop drives one over the backend event stream; tests drive it
/// directly with a constructed [`Event`], bypassing the socket.
#[derive(Clone)]
pub struct WebSocketHandler<B>
where
    B: Clone + Send + Sync + 'static,
    StoreCtx<B>: StoreView<WasiWebSocket>,
{
    state: Runtime<B>,
    component: String,
    routing: Arc<TriggerRouter<PatternRoutes>>,
}

impl<B> WebSocketHandler<B>
where
    B: Clone + Send + Sync + 'static,
    StoreCtx<B>: StoreView<WasiWebSocket>,
{
    /// Build the handler over `runtime`'s registry; `None` when no guest
    /// exports the websocket handler (the trigger is inert).
    ///
    /// # Errors
    ///
    /// Returns an error if the deployment's websocket routes are inconsistent
    /// with the guests' exports.
    pub fn new(runtime: &Runtime<B>) -> Result<Option<Self>> {
        // Capability probe: a guest exports the websocket handler exactly when
        // its typed indices resolve. Build the route router once, up front,
        // over the guests loaded at boot; a declared guest is probed when
        // its first event loads it.
        let routing = TriggerRouter::build(
            runtime.registry(),
            "websocket",
            runtime.registry().routes().websocket().clone(),
            DuplexIndices::new,
        )?;
        if routing.is_inert() {
            return Ok(None);
        }
        Ok(Some(Self {
            state: runtime.clone(),
            component: runtime.name().to_owned(),
            routing: Arc::new(routing),
        }))
    }

    /// Forward an event to the guest routed by its route key (or the
    /// catch-all guest when it carries none).
    ///
    /// An event with no matching route is dropped silently.
    ///
    /// # Errors
    ///
    /// Returns an error if the guest cannot be loaded at its first use or
    /// instantiated, traps, or times out.
    pub async fn handle(&self, event: Event) -> Result<()> {
        // Resolve the guest by the event's route; an event with no route falls
        // into the catch-all (sole exporter). A miss is dropped, not an error.
        let routed = event
            .route
            .as_deref()
            .map_or_else(|| self.routing.catch_all(), |route| self.routing.resolve(route));
        let Some(guest_id) = routed else {
            tracing::debug!("no route for websocket event; dropping");
            return Ok(());
        };
        // The routed identity is the deployment's, so a miss here is a first
        // use that failed to load or a lifecycle race (e.g. concurrent
        // deregistration) — an error, never a server panic.
        let guest = self
            .state
            .guest(guest_id)
            .await
            .with_context(|| format!("resolving the routed guest `{guest_id}`"))?;
        // The route says this guest handles websocket events; a declared
        // guest's export is checked here, at its first use, where a boot
        // guest's was checked at boot.
        let indices =
            DuplexIndices::new(guest.instance_pre()).map_err(anyhow::Error::from).with_context(
                || format!("routed guest `{guest_id}` does not export the websocket handler"),
            )?;

        let mut store_data = self.state.store();
        let event_res = store_data
            .view()
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
                let client = store.with(|mut store| store.get().view().ctx.connect()).await?;
                client.events().await
            })
            .await?
    }
}
