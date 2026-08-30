use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use futures::StreamExt;
use omnia::{PatternRoutes, Runtime, StoreCtx, StoreView, TriggerRouter};
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
    let component = state.name().to_owned();
    tracing::info!("starting websocket server for: {component}");

    // Capability probe: a guest exports the websocket handler exactly when its
    // typed indices resolve. Build the per-guest indices and the route router
    // that selects among them once, up front.
    let routing = TriggerRouter::build(
        state.registry(),
        "websocket",
        state.registry().routes().websocket().clone(),
        DuplexIndices::new,
    )?;
    if routing.is_inert() {
        tracing::info!("no guest exports the websocket handler; websocket trigger inert");
        return Ok(());
    }

    let handler = Handler {
        state: state.clone(),
        component,
        routing: Arc::new(routing),
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

#[derive(Clone)]
struct Handler<B>
where
    B: Clone + Send + Sync + 'static,
    StoreCtx<B>: StoreView<WasiWebSocket>,
{
    state: Runtime<B>,
    component: String,
    routing: Arc<TriggerRouter<DuplexIndices, PatternRoutes>>,
}

impl<B> Handler<B>
where
    B: Clone + Send + Sync + 'static,
    StoreCtx<B>: StoreView<WasiWebSocket>,
{
    /// Forward event to the wasm guest.
    async fn handle(&self, event: Event) -> Result<()> {
        // Resolve the guest by the event's route; an event with no route falls
        // into the catch-all (sole exporter). A miss is dropped, not an error.
        let routed = event
            .route
            .as_deref()
            .map_or_else(|| self.routing.catch_all(), |route| self.routing.resolve(route));
        let Some((guest_id, indices)) = routed else {
            tracing::debug!("no route for websocket event; dropping");
            return Ok(());
        };
        // Static resolution only yields identities drawn from the registry, so
        // a miss is a lifecycle race (e.g. concurrent deregistration) — an
        // error, never a server panic.
        let guest = self
            .state
            .registry()
            .get(guest_id)
            .with_context(|| format!("routed guest `{guest_id}` is not registered"))?;

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
