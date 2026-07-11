use std::env;
use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use futures::StreamExt;
use omnia::{PatternRoutes, Runtime, StoreCtx, TriggerRouter};
use tracing::{Instrument, debug_span, instrument};

use crate::host::WasiMessagingView;
use crate::host::generated::MessagingRequestReplyIndices;
use crate::host::resource::{Message, Subscriptions};

#[instrument("messaging-server", skip(state))]
pub async fn run<B>(state: &Runtime<B>) -> Result<()>
where
    B: Clone + Send + Sync + 'static,
    StoreCtx<B>: WasiMessagingView,
{
    let component = env::var("COMPONENT").unwrap_or_else(|_| "unknown".into());
    tracing::info!("starting messaging server for: {component}");

    // Capability probe: a guest exports the messaging handler exactly when its
    // typed indices resolve. Build the per-guest indices and the topic router
    // that selects among them once, up front.
    let routing = TriggerRouter::build(
        state.registry(),
        "messaging",
        state.registry().routes().messaging().clone(),
        MessagingRequestReplyIndices::new,
    )?;
    if routing.is_inert() {
        tracing::info!("no guest exports the messaging handler; messaging trigger inert");
        return Ok(());
    }

    let handler = Handler {
        state: state.clone(),
        component,
        routing: Arc::new(routing),
    };
    let mut stream = handler.subscriptions().await?;

    while let Some(message) = stream.next().await {
        let handler = handler.clone();
        tokio::spawn(async move {
            tracing::info!(monotonic_counter.message_counter = 1, service = %handler.component);

            let topic = message.topic.clone();
            if let Err(e) = handler.handle(message).await {
                tracing::error!("issue processing message: {e}");
                tracing::error!(
                    monotonic_counter.processing_errors = 1,
                    service = %handler.component,
                    topic = %topic,
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
    StoreCtx<B>: WasiMessagingView,
{
    state: Runtime<B>,
    component: String,
    routing: Arc<TriggerRouter<MessagingRequestReplyIndices, PatternRoutes>>,
}

impl<B> Handler<B>
where
    B: Clone + Send + Sync + 'static,
    StoreCtx<B>: WasiMessagingView,
{
    // Forward message to the wasm guest.
    async fn handle(&self, message: Message) -> Result<()> {
        // Resolve the guest by topic; an unmatched topic is dropped, not an
        // error (the message simply has no handler in this deployment).
        let topic = message.topic.clone();
        let Some((guest_id, indices)) = self.routing.resolve(&topic) else {
            tracing::debug!(%topic, "no route for topic; dropping message");
            return Ok(());
        };
        let guest = self.state.registry().get(guest_id).expect("a capable guest is registered");

        let mut store_data = self.state.store();
        let msg_res = store_data
            .messaging()
            .table
            .push(message)
            .map_err(|e| anyhow!("failed to push message: {e}"))?;

        let mut store = self.state.build_store(store_data);
        let instance = self.state.instantiate(guest.instance_pre(), &mut store).await?;
        let messaging = indices.load(&mut store, &instance)?;

        let run = store
            .run_concurrent(async |store| {
                let guest = messaging.wasi_messaging_incoming_handler();
                guest
                    .call_handle(store, msg_res)
                    .await
                    .map(|_| ())
                    .map_err(anyhow::Error::from)
                    .context("issue sending message")
            })
            .instrument(debug_span!("messaging-handle"));

        tokio::time::timeout(self.state.options().guest_timeout, run)
            .await
            .context("messaging handler timed out")??
    }

    // Get subscriptions for the topics configured in the wasm component.
    async fn subscriptions(&self) -> Result<Subscriptions> {
        let store_data = self.state.store();
        let mut store = self.state.build_store(store_data);

        store
            .run_concurrent(async |store| {
                let client = store.with(|mut store| store.get().messaging().ctx.connect()).await?;
                client.subscribe().await
            })
            .await?
    }
}
