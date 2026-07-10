use std::error::Error;
use std::future::Future;

use crate::api::Provider;
use crate::api::invoke::CallContext;

/// A stateless application operation.
pub trait Operation<P: Provider>: Sized + Send + 'static {
    /// The typed operation input.
    type Input: Send + 'static;

    /// The typed operation output.
    type Output: Send + 'static;

    /// The operation failure.
    type Error: Error + Send + Sync + 'static;

    /// Execute the operation.
    fn call(
        input: Self::Input, context: CallContext<'_, P>,
    ) -> impl Future<Output = Result<Self::Output, Self::Error>> + Send;
}
