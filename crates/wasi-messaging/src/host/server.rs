use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use futures::StreamExt;
use omnia_core::{PatternRoutes, Runtime, StoreCtx, StoreView, TriggerRouter};
use tracing::{Instrument, info_span, instrument};

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

    // a server's subscription never ends on purpose: a backend that gives up
    // on its consumer ends the stream, and the process must exit for the
    // orchestrator to restart it rather than idle looking healthy
    Err(anyhow!("messaging subscription ended for: {}", state.name()))
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
    routing: Arc<TriggerRouter<PatternRoutes>>,
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
        // a guest is capable exactly when its typed indices resolve; a
        // declared guest is probed when its first message loads it
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
    /// The message's [`Ack`](crate::host::Ack) token, if any, is called once
    /// the guest has run — whether it returned `Ok` or `Err`, trapped, or
    /// timed out — and when there is no guest to run. It is dropped uncalled
    /// when the guest could not be loaded or instantiated.
    ///
    /// # Errors
    ///
    /// Returns an error if the guest cannot be loaded at its first use or
    /// instantiated, traps, times out, or returns an error.
    pub async fn handle(&self, mut message: Message) -> Result<()> {
        // the token stays host-side; the guest never sees it
        let ack = message.ack.take();

        // resolve the guest by topic; unmatched is dropped, not an error
        let topic = message.topic.clone();
        let Some(guest_id) = self.routing.resolve(&topic) else {
            tracing::debug!(%topic, "no route for topic; dropping message");
            if let Some(ack) = ack {
                ack.ack();
            }
            return Ok(());
        };

        // a routed guest that fails to load is an error, never a panic
        let guest = self
            .state
            .guest(guest_id)
            .await
            .with_context(|| format!("resolving the routed guest `{guest_id}`"))?;

        // a declared guest's export is checked here, at first use
        let indices = MessagingRequestReplyIndices::new(guest.instance_pre())
            .map_err(anyhow::Error::from)
            .with_context(|| {
                format!("routed guest `{guest_id}` does not export the messaging handler")
            })?;

        let mut store_data = self.state.store();
        let msg_res = store_data
            .view()
            .table
            .push(message)
            .map_err(|e| anyhow!("failed to push message: {e}"))?;

        let mut store = self.state.build_store(store_data);
        let instance = self.state.instantiate(guest.instance_pre(), &mut store).await?;
        let messaging = indices.load(&mut store, &instance)?;

        // the guest's own `Err` is a failed call, not a successful one
        let run = store
            .run_concurrent(async |store| {
                let guest = messaging.wasi_messaging_incoming_handler();
                guest
                    .call_handle(store, msg_res)
                    .await
                    .map_err(anyhow::Error::from)
                    .context("issue sending message")
                    .and_then(|res| res.map_err(|e| anyhow!("guest returned an error: {e}")))
            })
            .instrument(info_span!("messaging-handle"));

        // the guest ran, whatever it did with the message; the host neither
        // retries nor holds anything back, so every arm acks
        let result = tokio::time::timeout(self.state.options().guest_timeout, run)
            .await
            .context("messaging handler timed out")
            .and_then(|res| res.map_err(anyhow::Error::from))
            .and_then(|res| res);
        if let Some(ack) = ack {
            ack.ack();
        }
        result
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
