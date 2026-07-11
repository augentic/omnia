use std::fmt;
use std::sync::Arc;

use crate::api::Provider;
use crate::api::invocation::{Invocation, Metadata};
use crate::api::operation::Operation;

/// Context shared with an operation call.
#[derive(Clone, Copy, Debug)]
pub struct CallContext<'a, P: Provider> {
    /// The owning tenant or namespace.
    pub owner: &'a str,

    /// The provider used to fulfil the call.
    pub provider: &'a P,

    /// Transport-neutral invocation metadata.
    pub metadata: &'a Metadata,
}

/// Provider-owning operation invoker.
///
/// Clones share one provider allocation. Transports define its lifetime; HTTP
/// constructs one invoker per WASI request and keeps durable state host-side.
pub struct Invoker<P: Provider> {
    owner: Arc<str>,
    provider: Arc<P>,
}

impl<P: Provider> Invoker<P> {
    /// Create an invoker with one clone-shared provider allocation.
    pub fn new(owner: impl Into<String>, provider: P) -> Self {
        Self {
            owner: Arc::from(owner.into()),
            provider: Arc::new(provider),
        }
    }

    /// Invoke a stateless operation.
    ///
    /// # Errors
    ///
    /// Returns the operation's error.
    pub async fn invoke<O>(&self, invocation: Invocation<O::Input>) -> Result<O::Output, O::Error>
    where
        O: Operation<P>,
    {
        let context = CallContext {
            owner: self.owner.as_ref(),
            provider: self.provider.as_ref(),
            metadata: &invocation.metadata,
        };
        O::call(invocation.input, context).await
    }

    /// Return the owning tenant or namespace.
    #[must_use]
    pub fn owner(&self) -> &str {
        &self.owner
    }

    /// Return the shared provider.
    #[must_use]
    pub fn provider(&self) -> &P {
        &self.provider
    }
}

impl<P: Provider> Clone for Invoker<P> {
    fn clone(&self) -> Self {
        Self {
            owner: Arc::clone(&self.owner),
            provider: Arc::clone(&self.provider),
        }
    }
}

impl<P: Provider> fmt::Debug for Invoker<P> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Invoker").field("owner", &self.owner).finish_non_exhaustive()
    }
}
