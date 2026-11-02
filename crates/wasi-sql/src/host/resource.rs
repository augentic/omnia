use omnia_backend::sql::{Connection, DataType};

/// Proxy for a SQL connection, stored in the resource table.
pub type ConnectionProxy = omnia_core::Proxy<dyn Connection>;

/// Represents a statement resource in the WASI SQL host.
#[derive(Clone, Debug)]
pub struct Statement {
    /// SQL query string.
    pub query: String,

    /// Query parameters.
    pub params: Vec<DataType>,
}
