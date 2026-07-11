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

impl Metadata {
    /// Build metadata from a transport's named-value lookup.
    ///
    /// Names are the transport-neutral `request-id` / `correlation-id` /
    /// `causation-id`; the correlation id falls back to the request id.
    pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Self {
        let request_id = lookup("request-id");
        Self {
            correlation_id: lookup("correlation-id").or_else(|| request_id.clone()),
            request_id,
            causation_id: lookup("causation-id"),
            deadline: None,
        }
    }

    /// Mint metadata for a transport-initiated invocation.
    ///
    /// The freshly minted request id doubles as the correlation id.
    #[must_use]
    pub fn minted(request_id: String) -> Self {
        Self {
            correlation_id: Some(request_id.clone()),
            request_id: Some(request_id),
            causation_id: None,
            deadline: None,
        }
    }
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
