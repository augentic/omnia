use std::error::Error;
use std::marker::PhantomData;

use super::response::CommandResponse;

/// Converts parsed command arguments into an operation input.
pub trait Decoder<A, I, G>: Clone + Send + Sync + 'static {
    /// The conversion failure.
    type Error: Error + Send + Sync + 'static;

    /// Decode one parsed argument value.
    ///
    /// # Errors
    ///
    /// Returns an application-specific decoding failure.
    fn decode(&self, args: A, globals: &G) -> Result<I, Self::Error>;
}

impl<A, I, G, E, F> Decoder<A, I, G> for F
where
    E: Error + Send + Sync + 'static,
    F: Fn(A, &G) -> Result<I, E> + Clone + Send + Sync + 'static,
{
    type Error = E;

    fn decode(&self, args: A, globals: &G) -> Result<I, Self::Error> {
        self(args, globals)
    }
}

/// Uses the input type's explicit `TryFrom<A>` conversion.
#[derive(Clone, Copy, Debug, Default)]
pub struct TryIntoDecoder;

impl<A, I, G> Decoder<A, I, G> for TryIntoDecoder
where
    A: TryInto<I>,
    A::Error: Error + Send + Sync + 'static,
{
    type Error = A::Error;

    fn decode(&self, args: A, _globals: &G) -> Result<I, Self::Error> {
        args.try_into()
    }
}

pub use crate::api::Outcome;

/// Maps one typed route outcome onto command output.
pub trait Projector<T, O, D, G>: Clone + Send + Sync + 'static {
    /// A failure while rendering a typed outcome.
    type Error: Error + Send + Sync + 'static;

    /// Project a typed route outcome.
    ///
    /// # Errors
    ///
    /// Returns a rendering failure for application-local handling.
    fn project(
        &self, outcome: Outcome<T, O, D>, globals: &G,
    ) -> Result<CommandResponse, Self::Error>;

    /// Project a failure raised by `project`.
    fn project_failure(&self, error: Self::Error, globals: &G) -> CommandResponse;
}

/// Begins registration of one operation-backed command.
#[must_use]
pub fn run<A, O>() -> Run<A, O, TryIntoDecoder> {
    Run {
        about: None,
        long_about: None,
        aliases: Vec::new(),
        hidden: false,
        decoder: TryIntoDecoder,
        marker: PhantomData,
    }
}

/// A typed command route before its projector is bound.
pub struct Run<A, O, D> {
    pub(crate) about: Option<&'static str>,
    pub(crate) long_about: Option<&'static str>,
    pub(crate) aliases: Vec<&'static str>,
    pub(crate) hidden: bool,
    pub(crate) decoder: D,
    marker: PhantomData<fn(A) -> O>,
}

impl<A, O, D> Run<A, O, D> {
    /// Set the route's short help.
    #[must_use]
    pub const fn about(mut self, about: &'static str) -> Self {
        self.about = Some(about);
        self
    }

    /// Set the route's detailed help.
    #[must_use]
    pub const fn long_about(mut self, long_about: &'static str) -> Self {
        self.long_about = Some(long_about);
        self
    }

    /// Add a route alias.
    #[must_use]
    pub fn alias(mut self, alias: &'static str) -> Self {
        self.aliases.push(alias);
        self
    }

    /// Hide the route from parent help and generated completions.
    #[must_use]
    pub const fn hidden(mut self) -> Self {
        self.hidden = true;
        self
    }

    /// Replace the default `TryInto` decoder.
    #[must_use]
    pub fn decode_with<D2>(self, decoder: D2) -> Run<A, O, D2> {
        Run {
            about: self.about,
            long_about: self.long_about,
            aliases: self.aliases,
            hidden: self.hidden,
            decoder,
            marker: PhantomData,
        }
    }

    /// Bind an application-local output policy.
    #[must_use]
    pub fn project_with<Q>(self, projector: Q) -> Binding<A, O, D, Q> {
        Binding {
            about: self.about,
            long_about: self.long_about,
            aliases: self.aliases,
            hidden: self.hidden,
            decoder: self.decoder,
            projector,
            marker: PhantomData,
        }
    }
}

/// A fully typed route ready for router registration.
pub struct Binding<A, O, D, Q> {
    pub(crate) about: Option<&'static str>,
    pub(crate) long_about: Option<&'static str>,
    pub(crate) aliases: Vec<&'static str>,
    pub(crate) hidden: bool,
    pub(crate) decoder: D,
    pub(crate) projector: Q,
    pub(crate) marker: PhantomData<fn(A) -> O>,
}
