use std::env;

use anyhow::{Context, Result, anyhow};
use futures::StreamExt;
use omnia::State;
use tracing::{Instrument, debug_span, instrument};
use wasmtime::Store;

use crate::host::WasiMessagingView;
use crate::host::generated::Messaging;
use crate::host::resource::{MessageProxy, Subscriptions};

#[instrument("messaging-server", skip(state))]
pub async fn run<S>(state: &S) -> Result<()>
where
    S: State,
    S::StoreCtx: WasiMessagingView,
{
    let component = env::var("COMPONENT").unwrap_or_else(|_| "unknown".into());
    tracing::info!("starting messaging server for: {component}");

    let handler = Handler {
        state: state.clone(),
        component,
    };
    let mut stream = handler.subscriptions().await?;

    while let Some(message) = stream.next().await {
        let handler = handler.clone();
        tokio::spawn(async move {
            tracing::info!(monotonic_counter.message_counter = 1, service = %handler.component);

            if let Err(e) = handler.handle(message.clone()).await {
                tracing::error!("issue processing message: {e}");
                tracing::error!(
                    monotonic_counter.processing_errors = 1,
                    service = %handler.component,
                    topic = %message.topic(),
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
    S::StoreCtx: WasiMessagingView,
{
    state: S,
    component: String,
}

impl<S> Handler<S>
where
    S: State,
    S::StoreCtx: WasiMessagingView,
{
    // Forward message to the wasm guest.
    async fn handle(&self, message: MessageProxy) -> Result<()> {
        let mut store_data = self.state.store();
        let msg_res = store_data
            .messaging()
            .table
            .push(message)
            .map_err(|e| anyhow!("failed to push message: {e}"))?;

        let instance_pre = self.state.instance_pre();
        let mut store = Store::new(instance_pre.engine(), store_data);
        let instance = instance_pre.instantiate_async(&mut store).await?;
        let messaging = Messaging::new(&mut store, &instance)?;

        store
            .run_concurrent(async |store| {
                let guest = messaging.wasi_messaging_incoming_handler();
                guest
                    .call_handle(store, msg_res)
                    .await
                    .map(|_| ())
                    .map_err(anyhow::Error::from)
                    .context("issue sending message")
            })
            .instrument(debug_span!("messaging-handle"))
            .await?
    }

    // Get subscriptions for the topics configured in the wasm component.
    async fn subscriptions(&self) -> Result<Subscriptions> {
        let instance_pre = self.state.instance_pre();
        let store_data = self.state.store();
        let mut store = Store::new(instance_pre.engine(), store_data);

        store
            .run_concurrent(async |store| {
                let client = store.with(|mut store| store.get().messaging().ctx.connect()).await?;
                client.subscribe().await
            })
            .await?
    }
}
