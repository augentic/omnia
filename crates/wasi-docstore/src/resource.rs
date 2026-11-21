//! The values a backend exchanges with the host.

/// A stored document, as a backend reads and writes it.
#[derive(Debug, Clone)]
pub struct Document {
    /// The document's id.
    pub id: String,
    /// The body, a JSON object encoded as UTF-8.
    pub data: Vec<u8>,
}

/// One page of a query, as a backend answers it.
#[derive(Debug, Clone)]
pub struct QueryResult {
    /// The documents on this page, in the order the query asked for.
    pub documents: Vec<Document>,
    /// The cursor that resumes the query, `None` on the last page.
    pub continuation: Option<String>,
}
