//! Filter resource proxy for `omnia:docstore` host bindings.

use omnia_backend::docstore::Filter;

/// Wrapper stored in the wasmtime resource table for `filter` handles.
#[derive(Debug, Clone)]
pub struct FilterProxy(pub Filter);
