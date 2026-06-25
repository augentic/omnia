use std::collections::HashMap;
use std::env;
use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use futures::StreamExt;
use omnia::{GuestId, Router, Runtime, TopicRouteTable};
use tracing::{Instrument, debug_span, instrument};

use crate::host::WasiMessagingView;
use crate::host::generated::MessagingRequestReplyIndices;
use crate::host::resource::{MessageProxy, Subscriptions};

#[instrument("messaging-server", skip(state))]
pub async fn run<S>(state: &S) -> Result<()>
where
    S: Runtime,
    S::StoreCtx: WasiMessagingView,
{
    let component = env::var("COMPONENT").unwrap_or_else(|_| "unknown".into());
    tracing::info!("starting messaging server for: {component}");

    // Capability probe: a guest exports the messaging handler exactly when its
    // typed indices resolve. Build the per-guest indices once and the topic
    // router that selects among them.
    let mut indices = HashMap::new();
    let mut capable = Vec::new();
    for guest in state.registry().guests() {
        if let Ok(messaging) = MessagingRequestReplyIndices::new(guest.instance_pre()) {
            capable.push(guest.id().clone());
            indices.insert(guest.id().clone(), messaging);
        }
    }
    let router =
        Router::build("messaging", &capable, state.registry().routes().messaging().clone())?;
    if router.is_inert() {
        tracing::info!("no guest exports the messaging handler; messaging trigger inert");
        return Ok(());
    }

    let handler = Handler {
        state: state.clone(),
        component,
        indices: Arc::new(indices),
        router: Arc::new(router),
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
    S: Runtime,
    S::StoreCtx: WasiMessagingView,
{
    state: S,
    component: String,
    indices: Arc<HashMap<GuestId, MessagingRequestReplyIndices>>,
    router: Arc<Router<TopicRouteTable>>,
}

impl<S> Handler<S>
where
    S: Runtime,
    S::StoreCtx: WasiMessagingView,
{
    // Forward message to the wasm guest.
    async fn handle(&self, message: MessageProxy) -> Result<()> {
        // Resolve the guest by topic; an unmatched topic is dropped, not an
        // error (the message simply has no handler in this deployment).
        let topic = message.topic();
        let Some(guest_id) = self.router.resolve(&topic) else {
            tracing::debug!(%topic, "no route for topic; dropping message");
            return Ok(());
        };
        let (Some(guest), Some(indices)) =
            (self.state.registry().get(guest_id), self.indices.get(guest_id))
        else {
            tracing::debug!(%topic, "routed guest is not registered or not messaging-capable");
            return Ok(());
        };

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
