use std::time::SystemTime;

/// Transport-neutral invocation metadata.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Metadata {
    /// Identifies this invocation at its transport boundary.
    pub request_id: Option<String>,

    /// Correlates work across transport and capability boundaries.
    pub correlation_id: Option<String>,

    /// Identifies the invocation that directly caused this work.
    pub causation_id: Option<String>,

    /// The latest instant at which the caller considers the work useful.
    pub deadline: Option<SystemTime>,
}

/// One typed operation invocation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Invocation<I> {
    /// The operation input.
    pub input: I,

    /// Metadata supplied by the invoking transport.
    pub metadata: Metadata,
}

impl<I> Invocation<I> {
    /// Create an invocation without metadata.
    pub fn new(input: I) -> Self {
        Self {
            input,
            metadata: Metadata::default(),
        }
    }

    /// Attach transport-neutral metadata.
    #[must_use]
    pub fn metadata(mut self, metadata: Metadata) -> Self {
        self.metadata = metadata;
        self
    }
}
