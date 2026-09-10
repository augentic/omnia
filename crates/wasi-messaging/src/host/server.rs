use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use futures::StreamExt;
use omnia_core::{PatternRoutes, Runtime, StoreCtx, StoreView, TriggerRouter};
use tracing::{Instrument, debug_span, instrument};

use crate::host::WasiMessaging;
use crate::host::generated::MessagingRequestReplyIndices;
use crate::host::resource::{Message, Subscriptions};

#[instrument("messaging-server", skip(state))]
pub async fn run<B>(state: &Runtime<B>) -> Result<()>
where
    B: Clone + Send + Sync + 'static,
    StoreCtx<B>: StoreView<WasiMessaging>,
{
    tracing::info!("starting messaging server for: {}", state.name());

    let Some(handler) = MessagingHandler::new(state)? else {
        tracing::info!("no guest exports the messaging handler; messaging trigger inert");
        return Ok(());
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

/// In-process `wasi:messaging/incoming-handler` dispatcher: routes a message
/// to a guest by topic, instantiates it, and invokes the export.
///
/// The server loop drives one over the backend subscription; tests drive it
/// directly with a constructed [`Message`], bypassing the broker.
#[derive(Clone)]
pub struct MessagingHandler<B>
where
    B: Clone + Send + Sync + 'static,
    StoreCtx<B>: StoreView<WasiMessaging>,
{
    state: Runtime<B>,
    component: String,
    routing: Arc<TriggerRouter<MessagingRequestReplyIndices, PatternRoutes>>,
}

impl<B> MessagingHandler<B>
where
    B: Clone + Send + Sync + 'static,
    StoreCtx<B>: StoreView<WasiMessaging>,
{
    /// Build the handler over `runtime`'s registry; `None` when no guest
    /// exports the messaging handler (the trigger is inert).
    ///
    /// # Errors
    ///
    /// Returns an error if the deployment's messaging routes are inconsistent
    /// with the guests' exports.
    pub fn new(runtime: &Runtime<B>) -> Result<Option<Self>> {
        // Capability probe: a guest exports the messaging handler exactly when
        // its typed indices resolve. Build the per-guest indices and the topic
        // router that selects among them once, up front.
        let routing = TriggerRouter::build(
            runtime.registry(),
            "messaging",
            runtime.registry().routes().messaging().clone(),
            MessagingRequestReplyIndices::new,
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

    /// Forward a message to the guest routed by its topic.
    ///
    /// A topic with no route is dropped silently; the message has no handler
    /// in this deployment.
    ///
    /// # Errors
    ///
    /// Returns an error if the guest cannot be instantiated, traps, or times
    /// out.
    pub async fn handle(&self, message: Message) -> Result<()> {
        // Resolve the guest by topic; an unmatched topic is dropped, not an
        // error (the message simply has no handler in this deployment).
        let topic = message.topic.clone();
        let Some((guest_id, indices)) = self.routing.resolve(&topic) else {
            tracing::debug!(%topic, "no route for topic; dropping message");
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
        let msg_res = store_data
            .view()
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
                let client = store.with(|mut store| store.get().view().ctx.connect()).await?;
                client.subscribe().await
            })
            .await?
    }
}
