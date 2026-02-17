use std::env;

use anyhow::{Context, Result, anyhow};
use futures::StreamExt;
use qwasr::State;
use tracing::{Instrument, debug_span, instrument};
use wasmtime::Store;

use crate::host::WebSocketView;
use crate::host::generated::Websocket;
use crate::host::resource::{EventProxy, Subscriptions};

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
    };
    let mut stream = handler.subscriptions().await?;

    println!("server::run");

    while let Some(event) = stream.next().await {
        println!("event received: {event:?}");

        let handler = handler.clone();
        tokio::spawn(async move {
            tracing::info!(monotonic_counter.event_counter = 1, service = %handler.component);

            if let Err(e) = handler.handle(event.clone()).await {
                tracing::error!("issue processing event: {e}");
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

        let instance_pre = self.state.instance_pre();
        let mut store = Store::new(instance_pre.engine(), store_data);
        let instance = instance_pre.instantiate_async(&mut store).await?;
        let websocket = Websocket::new(&mut store, &instance)?;

        println!("server::handle");

        store
            .run_concurrent(async |store| {
                let guest = websocket.wasi_websocket_handler();
                guest
                    .call_handle(store, event_res)
                    .await
                    .map(|_| ())
                    .map_err(anyhow::Error::from)
                    .context("issue handling event")
            })
            .instrument(debug_span!("websocket-handle"))
            .await?
    }

    /// Get subscriptions for incoming WebSocket events.
    async fn subscriptions(&self) -> Result<Subscriptions> {
        let instance_pre = self.state.instance_pre();
        let store_data = self.state.store();
        let mut store = Store::new(instance_pre.engine(), store_data);

        store
            .run_concurrent(async |store| {
                let socket = store.with(|mut store| store.get().websocket().ctx.connect()).await?;
                socket.subscribe().await
            })
            .await?
    }
}
