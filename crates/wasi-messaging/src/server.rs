//! The messaging trigger, whose [`MessagingHandler`] routes each delivery to a
//! guest by topic.

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Result, anyhow};
use futures::FutureExt as _;
use futures::future::BoxFuture;
use omnia_core::{ChainCtx, PatternRoutes, Provides, Runtime, TriggerRouter, telemetry};
use tracing::{Instrument, info_span, instrument};

use crate::WasiMessaging;
use crate::generated::MessagingIndices;
use crate::generated::omnia::messaging::types::Delivery;
use crate::resource::{Endpoint, HandleError, Handler, Message};

#[instrument("messaging-server", skip(state))]
pub async fn run<B>(state: &Runtime<B>) -> Result<()>
where
    B: Provides<WasiMessaging>,
{
    tracing::info!("starting messaging server for: {}", state.name());

    let Some(handler) = MessagingHandler::new(state)? else {
        tracing::info!("no guest exports the messaging handler; messaging trigger inert");
        return Ok(());
    };
    let client = handler.client().await?;
    client.consume(Arc::new(handler)).await?;

    // a consumer never ends on purpose: a backend that gives up on its
    // transport returns, and the process must exit for the orchestrator to
    // restart it rather than idle looking healthy
    Err(anyhow!("messaging consumer ended for: {}", state.name()))
}

/// In-process `omnia:messaging/incoming-handler` dispatcher: routes a message
/// to a guest by topic, instantiates it, and invokes the export.
///
/// The server hands one to the backend's [`Endpoint::consume`] as the
/// [`Handler`]; tests drive it directly with a constructed [`Message`],
/// bypassing the broker.
#[derive(Clone)]
pub struct MessagingHandler<B>
where
    B: Provides<WasiMessaging>,
{
    state: Runtime<B>,
    component: Arc<str>,
    routing: Arc<TriggerRouter<PatternRoutes>>,
}

impl<B> MessagingHandler<B>
where
    B: Provides<WasiMessaging>,
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
            MessagingIndices::new,
        )?;
        if routing.is_inert() {
            return Ok(None);
        }
        Ok(Some(Self {
            state: runtime.clone(),
            component: Arc::from(runtime.name()),
            routing: Arc::new(routing),
        }))
    }

    /// Forward a message to the guest routed by its topic.
    ///
    /// A topic with no route is dropped silently with `Ok`; the message has
    /// no handler in this deployment. Every message is counted, and every
    /// failure logged and counted, before it is returned. A routed message
    /// waits for a slot of the runtime's [concurrency gate](Runtime::slot)
    /// before its guest loads, however long that takes. The guest receives
    /// the delivery with its `deadline` set to the instant the guest timeout
    /// expires, counted from when the slot was taken, and its metadata as the
    /// backend delivered it.
    ///
    /// # Errors
    ///
    /// Returns [`HandleError::Unavailable`] if the guest cannot be loaded at
    /// its first use or instantiated, [`HandleError::Rejected`] or
    /// [`HandleError::Released`] as the guest settles the message,
    /// [`HandleError::Trapped`] if it traps, and [`HandleError::TimedOut`] if
    /// it runs past the guest timeout.
    pub async fn handle(&self, message: Message) -> Result<(), HandleError> {
        telemetry::count_message(&self.component);

        let topic = message.topic.clone();
        let outcome = self.deliver(message).await;
        if let Err(error) = &outcome {
            telemetry::count_error(&self.component, Some(topic.as_str()));
            tracing::error!(
                service = %self.component,
                topic = %topic,
                error = %error,
                "issue processing message"
            );
        }
        outcome
    }

    async fn deliver(&self, message: Message) -> Result<(), HandleError> {
        // resolve the guest by topic; unmatched is dropped, not an error
        let Some(guest_id) = self.routing.resolve(&message.topic) else {
            tracing::debug!(topic = %message.topic, "no route for topic; dropping message");
            return Ok(());
        };

        // wait for a slot of the gate, held until after the store drops
        let _slot = self.state.slot().await;

        // a routed guest that fails to load is an error, never a panic
        let guest = self.state.guest(guest_id).await.map_err(|error| {
            HandleError::Unavailable(format!("resolving the routed guest `{guest_id}`: {error}"))
        })?;

        // a declared guest's export is checked here, at first use
        let indices = MessagingIndices::new(guest.instance_pre()).map_err(|error| {
            HandleError::Unavailable(format!(
                "routed guest `{guest_id}` does not export the messaging handler: {error:#}"
            ))
        })?;

        let delivery = Delivery {
            deadline: Some(deadline_millis(self.state.options().guest_timeout)),
            ..Delivery::from(message)
        };

        let (mut store, instance) =
            self.state.enter(&guest, ChainCtx::server()).await.map_err(|error| {
                HandleError::Unavailable(format!("instantiating `{guest_id}`: {error:#}"))
            })?;
        let messaging = indices
            .load(&mut store, &instance)
            .map_err(|error| HandleError::Unavailable(format!("{error:#}")))?;

        let run = store
            .run_concurrent(async |store| {
                messaging.omnia_messaging_incoming_handler().call_handle(store, delivery).await
            })
            .instrument(info_span!("messaging-handle"));

        // the guest's own `Err` is one outcome; a trap on either side of the
        // call is another; a hung guest is cancelled with its store
        let timeout = self.state.options().guest_timeout;
        let called = tokio::time::timeout(timeout, run)
            .await
            .map_err(|_elapsed| HandleError::TimedOut(timeout))?;
        let returned =
            called.flatten().map_err(|trap| HandleError::Trapped(format!("{trap:#}")))?;
        returned.map_err(HandleError::from)
    }

    async fn client(&self) -> Result<Arc<dyn Endpoint>> {
        let mut backends = self.state.backends().clone();
        let endpoint = Provides::<WasiMessaging>::borrow(&mut backends).endpoint();
        endpoint.await
    }
}

impl<B> Handler for MessagingHandler<B>
where
    B: Provides<WasiMessaging>,
{
    fn handle(&self, message: Message) -> BoxFuture<'static, Result<(), HandleError>> {
        let handler = self.clone();
        async move { handler.handle(message).await }.boxed()
    }
}

// saturates rather than panics, so no configured timeout can fail a delivery
fn deadline_millis(timeout: Duration) -> u64 {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    u64::try_from(now.saturating_add(timeout).as_millis()).unwrap_or(u64::MAX)
}
