//! Native server runner for HTTP and messaging triggers.

use std::fmt;
use std::future::Future;
use std::sync::Arc;

use anyhow::{Context as _, Result, anyhow};
use futures::FutureExt as _;
use futures::future::{BoxFuture, try_join_all};
use omnia_backend::messaging::{
    Error as MessagingError, HandleError, Handler, Message, MessagingClient,
};
#[cfg(feature = "telemetry")]
use omnia_backend::{Telemetry, telemetry};
use omnia_sdk::api::messaging::{Delivery, DeliveryError, Router as MessagingRouter};
use tokio::net::TcpListener;
use tokio::sync::watch;

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
            match router.handle(Delivery::from(message)).await {
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
    http: Vec<(String, axum::Router)>,
    messaging: Vec<BoxFuture<'static, Result<()>>>,
    shutdown: Option<BoxFuture<'static, ()>>,
    #[cfg(feature = "telemetry")]
    telemetry: Option<Telemetry>,
}

impl fmt::Debug for Serve {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let addrs: Vec<&str> = self.http.iter().map(|(addr, _)| addr.as_str()).collect();
        let mut debug = f.debug_struct("Serve");
        debug
            .field("http", &addrs)
            .field("messaging", &self.messaging.len())
            .field("shutdown", &self.shutdown.is_some());
        #[cfg(feature = "telemetry")]
        debug.field("telemetry", &self.telemetry.is_some());
        debug.finish()
    }
}

impl Serve {
    /// Create an empty server builder.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Install process telemetry before starting the server triggers.
    #[cfg(feature = "telemetry")]
    #[must_use]
    pub fn telemetry(mut self, telemetry: Telemetry) -> Self {
        self.telemetry = Some(telemetry);
        self
    }

    /// Serve `router` over HTTP bound to `addr`.
    #[must_use]
    pub fn http(mut self, addr: impl Into<String>, router: axum::Router) -> Self {
        self.http.push((addr.into(), router));
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
        self.messaging.push(
            async move {
                let endpoint = client.endpoint().await.context("connecting messaging endpoint")?;
                endpoint.consume(handler).await.context("consuming messages")?;
                Err(anyhow!("messaging consumer ended"))
            }
            .boxed(),
        );
        self
    }

    /// Stop all triggers gracefully when `signal` resolves.
    ///
    /// Defaults to `SIGINT` (`Ctrl+C`) and `SIGTERM` (on Unix) when not set.
    #[must_use]
    pub fn shutdown(mut self, signal: impl Future<Output = ()> + Send + 'static) -> Self {
        self.shutdown = Some(signal.boxed());
        self
    }

    /// Run all configured triggers until shutdown or until one fails.
    ///
    /// # Errors
    ///
    /// Returns an error if telemetry initialization fails, an HTTP listener
    /// cannot bind, or a trigger exits with an error before shutdown.
    pub async fn run(self) -> Result<()> {
        #[cfg(feature = "telemetry")]
        if let Some(telemetry) = self.telemetry {
            telemetry.build()?;
        }

        if self.http.is_empty() && self.messaging.is_empty() {
            #[cfg(feature = "telemetry")]
            telemetry::flush();
            return Ok(());
        }

        let signal = self.shutdown.unwrap_or_else(|| default_shutdown().boxed());
        let (stop_tx, stop_rx) = watch::channel(false);
        tokio::spawn(async move {
            signal.await;
            let _ = stop_tx.send(true);
        });

        let mut tasks: Vec<BoxFuture<'static, Result<()>>> =
            Vec::with_capacity(self.http.len() + self.messaging.len());
        for (addr, router) in self.http {
            let stop = wait_for_stop(stop_rx.clone());
            tasks.push(
                async move {
                    let listener = TcpListener::bind(&addr)
                        .await
                        .with_context(|| format!("binding {addr}"))?;
                    axum::serve(listener, router)
                        .with_graceful_shutdown(stop)
                        .await
                        .context("serving http")
                }
                .boxed(),
            );
        }
        for consumer in self.messaging {
            let stop = wait_for_stop(stop_rx.clone());
            tasks.push(
                async move {
                    tokio::select! {
                        res = consumer => res,
                        () = stop => Ok(()),
                    }
                }
                .boxed(),
            );
        }

        let result = try_join_all(tasks).await.map(drop);
        #[cfg(feature = "telemetry")]
        telemetry::flush();
        result
    }
}

async fn wait_for_stop(mut rx: watch::Receiver<bool>) {
    let _ = rx.wait_for(|&stopped| stopped).await;
}

async fn default_shutdown() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        let terminate = async {
            if let Ok(mut sig) = signal(SignalKind::terminate()) {
                let _ = sig.recv().await;
            } else {
                std::future::pending::<()>().await;
            }
        };
        tokio::select! {
            () = ctrl_c => {}
            () = terminate => {}
        }
    }
    #[cfg(not(unix))]
    ctrl_c.await;
}
