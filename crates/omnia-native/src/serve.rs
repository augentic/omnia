//! Native server runner for HTTP and messaging triggers.

use std::fmt;
use std::sync::Arc;

use anyhow::{Context as _, Result, anyhow};
use futures::FutureExt as _;
use futures::future::BoxFuture;
use omnia_backend::messaging::{
    Error as MessagingError, HandleError, Handler, Message, MessagingClient,
};
use omnia_backend::{Telemetry, telemetry};
use omnia_sdk::api::messaging::{Delivery, DeliveryError, Router as MessagingRouter};
use tokio::net::TcpListener;

/// Adapts an [`omnia_sdk::api::messaging::Router`] to [`omnia_backend::messaging::Handler`].
pub struct RouterHandler<P: Send + Sync + 'static> {
    router: Arc<MessagingRouter<P>>,
}

impl<P: Send + Sync + 'static> Clone for RouterHandler<P> {
    fn clone(&self) -> Self {
        Self {
            router: Arc::clone(&self.router),
        }
    }
}

impl<P: Send + Sync + 'static> fmt::Debug for RouterHandler<P> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RouterHandler").finish_non_exhaustive()
    }
}

impl<P: Send + Sync + 'static> RouterHandler<P> {
    /// Wrap a messaging [`Router`](MessagingRouter) as a backend [`Handler`].
    #[must_use]
    pub fn new(router: MessagingRouter<P>) -> Self {
        Self {
            router: Arc::new(router),
        }
    }
}

impl<P: Send + Sync + 'static> Handler for RouterHandler<P> {
    fn handle(&self, message: Message) -> BoxFuture<'static, Result<(), HandleError>> {
        let router = Arc::clone(&self.router);
        async move {
            let content_type =
                message.metadata.as_ref().and_then(|md| md.get("content-type").cloned());
            let metadata =
                message.metadata.map(|md| md.inner.into_iter().collect()).unwrap_or_default();
            let delivery = Delivery {
                topic: (!message.topic.is_empty()).then_some(message.topic),
                payload: message.payload,
                content_type,
                metadata,
            };
            match router.handle(delivery).await {
                Ok(()) | Err(DeliveryError::UnhandledTopic(_)) => Ok(()),
                Err(error) => Err(HandleError::Rejected(MessagingError::Other(error.to_string()))),
            }
        }
        .boxed()
    }
}

/// Native server runner driving HTTP and messaging triggers in-process.
#[derive(Default)]
pub struct Serve {
    http: Option<(String, axum::Router)>,
    messaging: Option<BoxFuture<'static, Result<()>>>,
    telemetry: Option<Telemetry>,
}

impl fmt::Debug for Serve {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Serve")
            .field("http", &self.http.as_ref().map(|(addr, _)| addr))
            .field("messaging", &self.messaging.is_some())
            .field("telemetry", &self.telemetry.is_some())
            .finish()
    }
}

impl Serve {
    /// Create an empty server builder.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Install process telemetry before starting the server triggers.
    #[must_use]
    pub fn telemetry(mut self, telemetry: Telemetry) -> Self {
        self.telemetry = Some(telemetry);
        self
    }

    /// Serve `router` over HTTP bound to `addr`.
    #[must_use]
    pub fn http(mut self, addr: impl Into<String>, router: axum::Router) -> Self {
        self.http = Some((addr.into(), router));
        self
    }

    /// Consume inbound messages from `client` and dispatch them through `router`.
    #[must_use]
    pub fn messaging<M, P>(mut self, client: M, router: MessagingRouter<P>) -> Self
    where
        M: MessagingClient,
        P: Send + Sync + 'static,
    {
        let handler = Arc::new(RouterHandler::new(router));
        self.messaging = Some(
            async move {
                let endpoint = client.endpoint().await.context("connecting messaging endpoint")?;
                endpoint.consume(handler).await.context("consuming messages")?;
                Err(anyhow!("messaging consumer ended"))
            }
            .boxed(),
        );
        self
    }

    /// Run all configured triggers until one terminates or fails.
    ///
    /// # Errors
    ///
    /// Returns an error if telemetry initialization fails, the HTTP listener
    /// cannot bind, or a trigger exits with an error.
    pub async fn run(self) -> Result<()> {
        if let Some(telemetry) = self.telemetry {
            telemetry.build()?;
        }

        let http_fut = async move {
            let Some((addr, router)) = self.http else {
                return Ok(());
            };
            let listener =
                TcpListener::bind(&addr).await.with_context(|| format!("binding {addr}"))?;
            axum::serve(listener, router).await.context("serving http")
        };

        let messaging_fut = async move {
            let Some(messaging) = self.messaging else {
                return Ok(());
            };
            messaging.await
        };

        let result = tokio::try_join!(http_fut, messaging_fut).map(|((), ())| ());
        telemetry::flush();
        result
    }
}
