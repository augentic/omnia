use std::env;

use anyhow::{Context, Result, anyhow};
use futures::StreamExt;
use omnia::State;
use tracing::{Instrument, debug_span, instrument};

use crate::host::WebSocketView;
use crate::host::generated::DuplexIndices;
use crate::host::resource::{EventProxy, Events};

#[instrument("websocket-server", skip(state))]
pub async fn run<S>(state: &S) -> Result<()>
where
    S: State,
    S::StoreCtx: WebSocketView,
{
    let component = env::var("COMPONENT").unwrap_or_else(|_| "unknown".into());
    tracing::info!("starting websocket server for: {component}");

    let handler = Handler {
        state: state.clone(),
        component,
        indices: DuplexIndices::new(state.instance_pre())?,
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
    S: State,
    S::StoreCtx: WebSocketView,
{
    state: S,
    component: String,
    indices: DuplexIndices,
}

impl<S> Handler<S>
where
    S: State,
    S::StoreCtx: WebSocketView,
{
    /// Forward event to the wasm guest.
    async fn handle(&self, event: EventProxy) -> Result<()> {
        let mut store_data = self.state.store();
        let event_res = store_data
            .websocket()
            .table
            .push(event)
            .map_err(|e| anyhow!("failed to push event: {e}"))?;

        let mut store = self.state.build_store(store_data);
        let instance = self.state.instantiate(&mut store).await?;
        let websocket = self.indices.load(&mut store, &instance)?;

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
