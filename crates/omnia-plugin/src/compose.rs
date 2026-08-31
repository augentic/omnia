//! Acquirer composition: route each location to the first acquirer that
//! serves it.

use futures::FutureExt as _;
use futures::future::BoxFuture;

use crate::{Acquire, AcquireError, Location};

/// Fall-through composition for every [`Acquire`] value.
pub trait AcquireExt: Acquire + Sized {
    /// Serve locations with `self` first, falling through to `next` only
    /// when `self` does not support the location kind — a real failure
    /// never falls through.
    fn or<B: Acquire>(self, next: B) -> Or<Self, B> {
        Or {
            first: self,
            second: next,
        }
    }
}

impl<A: Acquire> AcquireExt for A {}

/// Two acquirers composed by location support (see [`AcquireExt::or`]).
#[derive(Clone, Copy, Debug)]
pub struct Or<A, B> {
    first: A,
    second: B,
}

impl<A: Acquire, B: Acquire> Acquire for Or<A, B> {
    fn acquire<'a>(
        &'a self, package: &'a str, from: &'a Location,
    ) -> BoxFuture<'a, Result<Vec<u8>, AcquireError>> {
        async move {
            match self.first.acquire(package, from).await {
                Err(AcquireError::Unsupported(first_refusal)) => {
                    match self.second.acquire(package, from).await {
                        Err(AcquireError::Unsupported(second_refusal)) => Err(
                            AcquireError::Unsupported(format!("{first_refusal}; {second_refusal}")),
                        ),
                        outcome => outcome,
                    }
                }
                outcome => outcome,
            }
        }
        .boxed()
    }
}
