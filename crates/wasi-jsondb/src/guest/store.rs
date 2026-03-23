//! High-level store API for guests (SDK `DocumentStore` delegates here).

use anyhow::Result;

use super::convert;
use super::generated::wasi::jsondb::store as wit_store;
use crate::document_store as sdk;

/// Fetch a document by id, if present.
///
/// # Errors
///
/// Returns an error when the host store call fails.
pub async fn get(collection: &str, id: &str) -> Result<Option<sdk::Document>> {
    let result = wit_store::get(collection.to_string(), id.to_string())
        .await
        .map_err(|e| anyhow::anyhow!("get failed: {e:?}"))?;
    Ok(result.map(convert::from_wit_document))
}

/// Insert a new document; fails if the id already exists (backend-defined).
///
/// # Errors
///
/// Returns an error when the host store call fails.
pub async fn insert(collection: &str, doc: &sdk::Document) -> Result<()> {
    wit_store::insert(collection.to_string(), convert::to_wit_document(doc))
        .await
        .map_err(|e| anyhow::anyhow!("insert failed: {e:?}"))
}

/// Upsert a document by id.
///
/// # Errors
///
/// Returns an error when the host store call fails.
pub async fn put(collection: &str, doc: &sdk::Document) -> Result<()> {
    wit_store::put(collection.to_string(), convert::to_wit_document(doc))
        .await
        .map_err(|e| anyhow::anyhow!("put failed: {e:?}"))
}

/// Delete a document by id. Returns whether a row was removed.
///
/// # Errors
///
/// Returns an error when the host store call fails.
pub async fn delete(collection: &str, id: &str) -> Result<bool> {
    wit_store::delete(collection.to_string(), id.to_string())
        .await
        .map_err(|e| anyhow::anyhow!("delete failed: {e:?}"))
}

/// Run a query with options.
///
/// # Errors
///
/// Returns an error when the host store call fails.
pub async fn query(collection: &str, options: sdk::QueryOptions) -> Result<sdk::QueryResult> {
    let wit_options = convert::to_wit_query_options(options);
    let result = wit_store::query(collection.to_string(), wit_options)
        .await
        .map_err(|e| anyhow::anyhow!("query failed: {e:?}"))?;
    Ok(convert::from_wit_query_result(result))
}
