//! The messaging trigger, whose [`MessagingHandler`] routes each delivery to a
//! guest by topic.

use std::num::NonZeroUsize;
use std::sync::Arc;

use anyhow::{Result, anyhow};
use futures::FutureExt as _;
use futures::future::BoxFuture;
use omnia_core::{LeaseError, PatternRoutes, Provides, Runtime, TriggerRouter, telemetry};
use tokio::sync::Semaphore;
use tracing::{Instrument, info_span, instrument};
use wasmtime::AsContextMut as _;

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
    let consumed = client.consume(Arc::new(handler)).await;
    if state.stopped().now_or_never().is_some() {
        return consumed;
    }
    consumed?;

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
/// bypassing the broker. Its [`stopped`](Handler::stopped) resolves once the
/// runtime [stops](Runtime::stop).
#[derive(Clone)]
pub struct MessagingHandler<B>
where
    B: Provides<WasiMessaging>,
{
    state: Runtime<B>,
    component: Arc<str>,
    routing: Arc<TriggerRouter<PatternRoutes>>,
    in_flight: NonZeroUsize,
    // messaging's share of the gate: a delivery past it waits here, off the
    // gate, so a backlog never shuts out HTTP or WebSocket
    share: Arc<Semaphore>,
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
        let in_flight = runtime.options().messaging_in_flight();
        Ok(Some(Self {
            state: runtime.clone(),
            component: Arc::from(runtime.name()),
            routing: Arc::new(routing),
            in_flight,
            share: Arc::new(Semaphore::new(in_flight.get())),
        }))
    }

    /// Forward a message to the guest routed by its topic.
    ///
    /// A topic with no route is dropped silently with `Ok`; the message has
    /// no handler in this deployment. Every message is counted, and every
    /// failure logged and counted, before it is returned. A routed message
    /// loads its guest, then waits for a slot of the runtime's
    /// [concurrency gate](Runtime::lease), however long that takes. No more
    /// than [`max_in_flight`](Handler::max_in_flight) messages load, wait for
    /// a slot, or hold one at once, and one past them waits its turn off the
    /// gate. The guest receives the delivery with its `deadline` set to the
    /// instant the guest timeout expires, counted from when the slot was
    /// taken, and its metadata as the backend delivered it.
    ///
    /// # Errors
    ///
    /// Returns [`HandleError::Unavailable`] if the guest cannot be loaded at
    /// its first use or instantiated, [`HandleError::Rejected`] or
    /// [`HandleError::Released`] as the guest settles the message,
    /// [`HandleError::Trapped`] if it traps, and [`HandleError::TimedOut`] if
    /// its instantiation and run outlast the guest timeout.
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

        // wait for a place in messaging's share, off the gate
        let _place = self.share.acquire().await.expect("the share is never closed");

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

        // a loaded guest waits for a slot of the gate, and its deadline counts
        // from the slot
        let lease = self.state.lease(guest).await;
        let delivery = Delivery {
            deadline: lease.deadline_millis(),
            ..Delivery::from(message)
        };

        // the guest's own `Err` is one outcome and a trap on either side of the
        // call another, and a hung guest is cancelled with its store
        let handled = lease
            .invoke(async |mut entered, instance, _bound| {
                let messaging = indices
                    .load(&mut entered, &instance)
                    .map_err(|error| HandleError::Unavailable(format!("{error:#}")))?;
                let called = entered
                    .as_context_mut()
                    .run_concurrent(async |store| {
                        messaging
                            .omnia_messaging_incoming_handler()
                            .call_handle(store, delivery)
                            .await
                    })
                    .await;
                let returned =
                    called.flatten().map_err(|trap| HandleError::Trapped(format!("{trap:#}")))?;
                returned.map_err(HandleError::from)
            })
            .instrument(info_span!("messaging-handle"))
            .await;
        match handled {
            Ok(handled) => handled,
            Err(LeaseError::Enter(error) | LeaseError::PoolFull(error)) => {
                Err(HandleError::Unavailable(format!("instantiating `{guest_id}`: {error:#}")))
            }
            Err(LeaseError::Timeout(limit)) => Err(HandleError::TimedOut(limit)),
            Err(error) => {
                Err(HandleError::Unavailable(format!("invoking `{guest_id}`: {error:#}")))
            }
        }
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

    fn max_in_flight(&self) -> NonZeroUsize {
        self.in_flight
    }

    fn stopped(&self) -> BoxFuture<'static, ()> {
        let state = self.state.clone();
        async move { state.stopped().await }.boxed()
    }
}
