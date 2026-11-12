use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Result, anyhow};
use futures::FutureExt as _;
use futures::future::BoxFuture;
use omnia_core::{PatternRoutes, Provides, Runtime, TriggerRouter};
use tokio::sync::Semaphore;
use tracing::{Instrument, info_span, instrument};

use crate::host::WasiMessaging;
use crate::host::generated::MessagingRequestReplyIndices;
use crate::host::resource::{Endpoint, HandleError, Handler, Message};

// the metadata entry omnia-sdk's `Metadata` reads a guest's deadline from
const DEADLINE: &str = "deadline";

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

/// In-process `wasi:messaging/incoming-handler` dispatcher: routes a message
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
    // one permit per running guest: whatever a backend delivers at once,
    // this trigger alone never exhausts the instance pool
    permits: Arc<Semaphore>,
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
            MessagingRequestReplyIndices::new,
        )?;
        if routing.is_inert() {
            return Ok(None);
        }
        Ok(Some(Self {
            state: runtime.clone(),
            component: Arc::from(runtime.name()),
            routing: Arc::new(routing),
            permits: Arc::new(Semaphore::new(runtime.options().pool_max_instances as usize)),
        }))
    }

    /// Forward a message to the guest routed by its topic.
    ///
    /// A topic with no route is dropped silently with `Ok`; the message has
    /// no handler in this deployment. Every message is counted, and every
    /// failure logged and counted, before it is returned. The guest receives
    /// the message with its `deadline` metadata entry set to the instant the
    /// guest timeout expires, in milliseconds since the Unix epoch, replacing
    /// any the backend delivered.
    ///
    /// # Errors
    ///
    /// Returns [`HandleError::Unavailable`] if the guest cannot be loaded at
    /// its first use or instantiated, [`HandleError::Rejected`] if it returns
    /// an error, [`HandleError::Trapped`] if it traps, and
    /// [`HandleError::TimedOut`] if it runs past the guest timeout.
    pub async fn handle(&self, message: Message) -> Result<(), HandleError> {
        tracing::info!(monotonic_counter.message_counter = 1, service = %self.component);

        let topic = message.topic.clone();
        let outcome = self.deliver(message).await;
        if let Err(error) = &outcome {
            tracing::error!(
                monotonic_counter.processing_errors = 1,
                service = %self.component,
                topic = %topic,
                error = %error,
                "issue processing message"
            );
        }
        outcome
    }

    async fn deliver(&self, mut message: Message) -> Result<(), HandleError> {
        // resolve the guest by topic; unmatched is dropped, not an error
        let Some(guest_id) = self.routing.resolve(&message.topic) else {
            tracing::debug!(topic = %message.topic, "no route for topic; dropping message");
            return Ok(());
        };

        let _permit = self.permits.acquire().await.expect("the pool semaphore is never closed");

        // a routed guest that fails to load is an error, never a panic
        let guest = self.state.guest(guest_id).await.map_err(|error| {
            HandleError::Unavailable(format!("resolving the routed guest `{guest_id}`: {error}"))
        })?;

        // a declared guest's export is checked here, at first use
        let indices = MessagingRequestReplyIndices::new(guest.instance_pre()).map_err(|error| {
            HandleError::Unavailable(format!(
                "routed guest `{guest_id}` does not export the messaging handler: {error:#}"
            ))
        })?;

        // stamp the guest's deadline over any the delivery carried, in any case
        let deadline = deadline_millis(self.state.options().guest_timeout);
        let metadata = message.metadata.get_or_insert_default();
        metadata.retain(|name, _| !name.eq_ignore_ascii_case(DEADLINE));
        metadata.insert(DEADLINE.to_owned(), deadline.to_string());

        let mut store_data = self.state.store();
        let msg_res =
            store_data.view::<WasiMessaging>().table.push(message).map_err(|error| {
                HandleError::Unavailable(format!("pushing the message: {error}"))
            })?;
        let mut store = self.state.build_store(store_data);
        let instance =
            self.state.instantiate(guest.instance_pre(), &mut store).await.map_err(|error| {
                HandleError::Unavailable(format!("instantiating `{guest_id}`: {error:#}"))
            })?;
        let messaging = indices
            .load(&mut store, &instance)
            .map_err(|error| HandleError::Unavailable(format!("{error:#}")))?;

        let run = store
            .run_concurrent(async |store| {
                messaging.wasi_messaging_incoming_handler().call_handle(store, msg_res).await
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
        returned.map_err(HandleError::Rejected)
    }

    async fn client(&self) -> Result<Arc<dyn Endpoint>> {
        self.state.store().view::<WasiMessaging>().client.endpoint().await
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
