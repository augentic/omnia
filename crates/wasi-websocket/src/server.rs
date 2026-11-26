//! The websocket trigger, whose [`WebSocketHandler`] routes each event to a
//! guest.

use std::sync::Arc;

use anyhow::{Context, Result, anyhow, bail};
use futures::StreamExt;
use omnia_core::{LeaseError, PatternRoutes, Provides, Runtime, TriggerRouter, telemetry};
use tokio::task::JoinSet;
use tracing::{Instrument, info_span, instrument};
use wasmtime::AsContextMut as _;

use crate::WasiWebSocket;
use crate::generated::WebsocketIndices;
use crate::generated::omnia::websocket::types;
use crate::resource::{Event, Events};

#[instrument("websocket-server", skip(state))]
pub async fn run<B>(state: &Runtime<B>) -> Result<()>
where
    B: Provides<WasiWebSocket>,
{
    tracing::info!("starting websocket server for: {}", state.name());

    let Some(handler) = WebSocketHandler::new(state)? else {
        tracing::info!("no guest exports the websocket handler; websocket trigger inert");
        return Ok(());
    };

    // subscribe once: a broadcast receiver only sees what arrives after it subscribes
    let mut events = handler.events().await?;
    let mut handling = JoinSet::new();

    // take events until the runtime stops: a stream that ends first has lost
    // its transport, and the process must exit for the orchestrator to
    // restart it rather than idle looking healthy
    loop {
        tokio::select! {
            biased;
            () = state.stopped() => break,
            Some(_) = handling.join_next() => {}
            event = events.next() => {
                let Some(event) = event else {
                    bail!("websocket event stream ended for: {}", state.name());
                };
                let handler = handler.clone();
                handling.spawn(async move {
                    _ = handler.handle(event).await;
                });
            }
        }
    }

    // the events taken before the stop finish before the server returns
    while handling.join_next().await.is_some() {}
    Ok(())
}

/// In-process `omnia:websocket/incoming-handler` dispatcher: routes an event
/// to a guest, instantiates it, and invokes the export.
///
/// The server loop drives one over the backend event stream; tests drive it
/// directly with a constructed [`Event`], bypassing the socket.
#[derive(Clone)]
pub struct WebSocketHandler<B>
where
    B: Provides<WasiWebSocket>,
{
    state: Runtime<B>,
    component: String,
    routing: Arc<TriggerRouter<PatternRoutes>>,
}

impl<B> WebSocketHandler<B>
where
    B: Provides<WasiWebSocket>,
{
    /// Build the handler over `runtime`'s registry; `None` when no guest
    /// exports the websocket handler (the trigger is inert).
    ///
    /// # Errors
    ///
    /// Returns an error if the deployment's websocket routes are inconsistent
    /// with the guests' exports.
    pub fn new(runtime: &Runtime<B>) -> Result<Option<Self>> {
        // a guest is capable exactly when its `WebsocketIndices` resolve; a
        // declared guest is probed when its first event loads it
        let routing = TriggerRouter::build(
            runtime.registry(),
            "websocket",
            runtime.registry().routes().websocket().clone(),
            WebsocketIndices::new,
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

    /// Forward an event to the guest routed by its route key (or the
    /// catch-all guest when it carries none).
    ///
    /// An event with no matching route is dropped silently. The guest
    /// receives the event with its `deadline` set to the instant the guest
    /// timeout expires, counted from when the event took its slot of the
    /// gate. A guest's error leaves the connection open.
    ///
    /// Every event is counted on the `event_counter` counter, and every
    /// failure logged and counted on `processing_errors`, before it is
    /// returned. An event refused at a full gate counts on
    /// `invocations_refused` alone, and is logged at `debug`.
    ///
    /// # Errors
    ///
    /// Returns an error if the event names no connection or carries text that
    /// is not UTF-8, if the guest cannot be loaded at its first use, if every
    /// slot of the runtime's [concurrency gate](Runtime::try_lease) is held,
    /// if the guest cannot be instantiated, traps, or times out, or if the
    /// guest returns an error.
    pub async fn handle(&self, event: Event) -> Result<()> {
        telemetry::count_event(&self.component);

        let handled = self.deliver(event).await;
        match &handled {
            Err(error) if error.is::<Refused>() => {
                tracing::debug!(service = %self.component, "{error}");
            }
            Err(error) => {
                telemetry::count_error(&self.component, None);
                tracing::error!(
                    service = %self.component,
                    error = format!("{error:#}"),
                    "issue processing event"
                );
            }
            Ok(()) => {}
        }
        handled
    }

    // `handle`'s outcome, with a full gate reported as `Refused`
    async fn deliver(&self, event: Event) -> Result<()> {
        // an event the guest could not answer is refused before routing
        let mut event = types::Event::try_from(event)
            .map_err(|error| anyhow!("refusing a websocket event: {error}"))?;

        // resolve the guest by route, or the catch-all; unmatched is dropped, not an error
        let routed = event
            .route
            .as_deref()
            .map_or_else(|| self.routing.catch_all(), |route| self.routing.resolve(route));
        let Some(guest_id) = routed else {
            tracing::debug!("no route for websocket event; dropping");
            return Ok(());
        };

        // a routed guest that fails to load is an error, never a panic
        let guest = self
            .state
            .guest(guest_id)
            .await
            .with_context(|| format!("resolving the routed guest `{guest_id}`"))?;

        // a declared guest's export is checked here, at first use
        let indices =
            WebsocketIndices::new(guest.instance_pre()).map_err(anyhow::Error::from).with_context(
                || format!("routed guest `{guest_id}` does not export the websocket handler"),
            )?;

        // a full gate fails the event, and its deadline counts from the slot
        let Some(lease) = self.state.try_lease(guest) else {
            return Err(Refused.into());
        };
        event.deadline = lease.deadline_millis();

        let handled = lease
            .invoke(async |mut entered, instance, _bound| {
                let websocket = indices.load(&mut entered, &instance)?;
                let called = entered
                    .as_context_mut()
                    .run_concurrent(async |store| {
                        websocket.omnia_websocket_incoming_handler().call_handle(store, event).await
                    })
                    .await;
                let returned = called
                    .flatten()
                    .map_err(anyhow::Error::from)
                    .context("issue handling event")?;
                returned.map_err(|error| anyhow!("the guest returned an error: {error}"))
            })
            .instrument(info_span!("websocket-handle"))
            .await;
        match handled {
            Ok(handled) => handled,
            Err(LeaseError::Enter(error) | LeaseError::PoolFull(error)) => Err(error),
            Err(timeout @ LeaseError::Timeout(_)) => {
                Err(anyhow::Error::from(timeout).context("websocket handler timed out"))
            }
            Err(error) => Err(error.into()),
        }
    }

    async fn events(&self) -> Result<Events> {
        let mut backends = self.state.backends().clone();
        let endpoint = Provides::<WasiWebSocket>::borrow(&mut backends).endpoint();
        endpoint.await?.events().await
    }
}

// an event refused at a full gate, which counts on `invocations_refused` alone
#[derive(Debug, thiserror::Error)]
#[error(
    "refusing a websocket event: every slot of the concurrency gate \
     (MAX_CONCURRENT_INVOCATIONS) is held"
)]
struct Refused;
