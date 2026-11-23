//! The websocket trigger, whose [`WebSocketHandler`] routes each event to a
//! guest.

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow};
use futures::StreamExt;
use omnia_core::{ChainCtx, PatternRoutes, Provides, Runtime, TriggerRouter, telemetry};
use tracing::{Instrument, info_span, instrument};

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

    while let Some(event) = events.next().await {
        let handler = handler.clone();

        tokio::spawn(async move {
            telemetry::count_event(&handler.component);

            if let Err(e) = handler.handle(event).await {
                telemetry::count_error(&handler.component, None);
                tracing::error!(service = %handler.component, error = %e);
            }
        });
    }

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
    /// timeout expires. A guest's error leaves the connection open.
    ///
    /// # Errors
    ///
    /// Returns an error if the event names no connection or carries text that
    /// is not UTF-8, if the guest cannot be loaded at its first use or
    /// instantiated, traps, or times out, or if the guest returns an error.
    pub async fn handle(&self, event: Event) -> Result<()> {
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

        event.deadline = Some(deadline_millis(self.state.options().guest_timeout));

        let (mut store, instance) = self.state.enter(&guest, ChainCtx::server()).await?;
        let websocket = indices.load(&mut store, &instance)?;

        let run = store
            .run_concurrent(async |store| {
                websocket.omnia_websocket_incoming_handler().call_handle(store, event).await
            })
            .instrument(info_span!("websocket-handle"));

        let called = tokio::time::timeout(self.state.options().guest_timeout, run)
            .await
            .context("websocket handler timed out")?;
        let returned =
            called.flatten().map_err(anyhow::Error::from).context("issue handling event")?;
        returned.map_err(|error| anyhow!("the guest returned an error: {error}"))
    }

    async fn events(&self) -> Result<Events> {
        let mut backends = self.state.backends().clone();
        let endpoint = Provides::<WasiWebSocket>::borrow(&mut backends).endpoint();
        endpoint.await?.events().await
    }
}

// saturates rather than panics, so no configured timeout can fail an event
fn deadline_millis(timeout: Duration) -> u64 {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    u64::try_from(now.saturating_add(timeout).as_millis()).unwrap_or(u64::MAX)
}
