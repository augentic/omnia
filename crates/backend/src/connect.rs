//! Construction lifecycle for backend clients and defaults.

use std::future::Future;

use anyhow::Result;
use futures::future::BoxFuture;

/// Result type for asynchronous client operations.
pub type FutureResult<T> = BoxFuture<'static, Result<T>>;

/// Connect a backend client or default resource during startup.
pub trait Connect: Sized + Sync + Send {
    /// The options used to connect to the backend.
    type ConnectOptions: FromEnv;

    /// Connect to the resource using options loaded from the environment.
    fn connect() -> impl Future<Output = Result<Self>> {
        async { Self::connect_with(Self::ConnectOptions::load_env()?).await }
    }

    /// Connect with the specified options.
    fn connect_with(options: Self::ConnectOptions) -> impl Future<Output = Result<Self>>;
}

/// Create backend connection options from environment variables.
///
/// The method is `load_env` (not `from_env`) so it never shadows — or is shadowed
/// by — the builder-returning inherent `from_env` the `fromenv` derive emits
/// on option structs.
pub trait FromEnv: Sized {
    /// Load connection options from environment variables.
    ///
    /// # Errors
    ///
    /// Returns an error if required environment variables are missing or invalid.
    fn load_env() -> Result<Self>;
}

/// Connection options for a [`Connect`] implementation that needs none.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoOptions;

impl FromEnv for NoOptions {
    fn load_env() -> Result<Self> {
        Ok(Self)
    }
}
