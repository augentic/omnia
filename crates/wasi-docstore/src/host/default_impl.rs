//! Default PoloDB-backed implementation of [`WasiDocStoreCtx`](crate::host::WasiDocStoreCtx).

mod bson_filter;

use std::sync::Arc;

use anyhow::{Context, Result};
use bson_filter::to_bson;
use futures::FutureExt;
use omnia::{Backend, FromEnv};
use polodb_core::bson::{self, doc};
use polodb_core::{CollectionT, Database};
use tracing::instrument;

use crate::host::generated::wasi::docstore::types::{Document, QueryResult, SortField};
use crate::host::resource::FilterTree;
use crate::host::{FutureResult, QueryOpts, WasiDocStoreCtx};

const MAX_PAGE_SIZE: u64 = 1000;

/// Connection options for the embedded `PoloDB` file.
#[derive(Debug, Clone)]
pub struct ConnectOptions {
    /// Filesystem path to the `PoloDB` database file.
    pub database: String,
}

impl FromEnv for ConnectOptions {
    fn from_env() -> Result<Self> {
        let database = std::env::var("DOCSTORE_DATABASE").unwrap_or_else(|_| {
            std::env::temp_dir().join("omnia-docstore.polodb").to_string_lossy().into_owned()
        });
        Ok(Self { database })
    }
}

/// Default [`WasiDocStoreCtx`] using `PoloDB`.
#[derive(Clone)]
pub struct DocStoreDefault {
    db: Arc<Database>,
}

impl std::fmt::Debug for DocStoreDefault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DocStoreDefault").field("db", &"<polodb Database>").finish()
    }
}

impl Backend for DocStoreDefault {
    type ConnectOptions = ConnectOptions;

    #[instrument]
    async fn connect_with(options: Self::ConnectOptions) -> Result<Self> {
        tracing::debug!("opening PoloDB at {}", options.database);
        let db = Database::open_path(&options.database).context("opening PoloDB database")?;
        Ok(Self { db: Arc::new(db) })
    }
}

impl WasiDocStoreCtx for DocStoreDefault {
    fn get(&self, collection: String, id: String) -> FutureResult<Option<Document>> {
        let db = Arc::clone(&self.db);
        async move {
            let col = db.collection::<bson::Document>(&collection);
            let found = col.find_one(doc! { "_id": id }).context("find_one in get")?;
            match found {
                Some(d) => Ok(Some(bson_to_wit_document(&d)?)),
                None => Ok(None),
            }
        }
        .boxed()
    }

    fn insert(&self, collection: String, doc: Document) -> FutureResult<()> {
        let db = Arc::clone(&self.db);
        async move {
            let col = db.collection::<bson::Document>(&collection);
            let bson_doc = wit_to_bson_document(&doc).context("encoding insert document")?;
            col.insert_one(bson_doc)
                .map_err(|e| match e {
                    polodb_core::Error::DuplicateKey(_) => {
                        anyhow::anyhow!("document id already exists")
                    }
                    other => anyhow::anyhow!("{other}"),
                })
                .context("insert_one")?;
            Ok(())
        }
        .boxed()
    }

    fn put(&self, collection: String, doc: Document) -> FutureResult<()> {
        let db = Arc::clone(&self.db);
        async move {
            let col = db.collection::<bson::Document>(&collection);
            let id = doc.id.clone();
            let bson_doc = wit_to_bson_document(&doc).context("encoding put document")?;
            let _ = col.delete_one(doc! { "_id": id }).context("delete before put")?;
            col.insert_one(bson_doc).context("insert after put delete")?;
            Ok(())
        }
        .boxed()
    }

    fn delete(&self, collection: String, id: String) -> FutureResult<bool> {
        let db = Arc::clone(&self.db);
        async move {
            let col = db.collection::<bson::Document>(&collection);
            let res = col.delete_one(doc! { "_id": id }).context("delete_one")?;
            Ok(res.deleted_count > 0_u64)
        }
        .boxed()
    }

    fn query(
        &self, collection: String, filter: Option<FilterTree>, options: QueryOpts,
    ) -> FutureResult<QueryResult> {
        let db = Arc::clone(&self.db);
        async move {
            if let Some(ref f) = filter {
                bson_filter::validate(f).context("invalid filter")?;
            }

            let limit = options.limit.map_or(MAX_PAGE_SIZE, u64::from);
            anyhow::ensure!(limit > 0, "query limit must be at least 1");
            let page_size = usize::try_from(limit).unwrap_or(usize::MAX);

            let col = db.collection::<bson::Document>(&collection);
            let bson_filter = filter.as_ref().map_or_else(|| doc! {}, to_bson);

            let skip_u64 = parse_continuation(options.continuation.as_deref())?
                + u64::from(options.offset.unwrap_or(0));

            let sort_doc = build_sort_document(&options.order_by);

            let mut find = col.find(bson_filter);
            if let Some(s) = sort_doc {
                find = find.sort(s);
            }
            find = find.skip(skip_u64);
            find = find.limit(limit.saturating_add(1));

            let cursor = find.run().context("find query")?;
            let mut raw = Vec::new();
            for item in cursor {
                raw.push(item.context("cursor item")?);
            }

            let has_more = raw.len() > page_size;
            if has_more {
                raw.truncate(page_size);
            }

            let mut documents = Vec::with_capacity(raw.len());
            for d in raw {
                documents.push(bson_to_wit_document(&d).context("decode query row")?);
            }

            let continuation = has_more.then(|| {
                let next = skip_u64.saturating_add(u64::try_from(documents.len()).unwrap_or(0));
                next.to_string()
            });

            Ok(QueryResult {
                documents,
                continuation,
            })
        }
        .boxed()
    }
}

fn parse_continuation(continuation: Option<&str>) -> Result<u64> {
    continuation.map_or_else(
        || Ok(0),
        |s| s.parse().with_context(|| format!("invalid continuation token: {s:?}")),
    )
}

fn build_sort_document(order_by: &[SortField]) -> Option<bson::Document> {
    if order_by.is_empty() {
        return None;
    }
    let mut doc = bson::Document::new();
    for f in order_by {
        let dir = if f.descending { -1 } else { 1 };
        doc.insert(f.field.clone(), dir);
    }
    Some(doc)
}

fn wit_to_bson_document(doc: &Document) -> Result<bson::Document> {
    let v: serde_json::Value =
        serde_json::from_slice(&doc.data).context("invalid JSON in document body")?;
    let mut bson_doc = bson::to_document(&v).context("JSON to BSON")?;
    bson_doc.insert("_id", doc.id.clone());
    Ok(bson_doc)
}

fn bson_to_wit_document(d: &bson::Document) -> Result<Document> {
    let id = match d.get("_id") {
        Some(bson::Bson::String(s)) => s.clone(),
        Some(bson::Bson::Int32(i)) => i.to_string(),
        Some(bson::Bson::Int64(i)) => i.to_string(),
        Some(bson::Bson::ObjectId(oid)) => oid.to_hex(),
        None => anyhow::bail!("stored document missing _id"),
        Some(_) => anyhow::bail!("unsupported _id type in stored document"),
    };

    let mut json_val = serde_json::to_value(d).context("BSON to JSON")?;
    if let serde_json::Value::Object(ref mut m) = json_val {
        m.remove("_id");
    }
    let data = serde_json::to_vec(&json_val).context("serialize JSON body")?;
    Ok(Document { id, data })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::bson_filter::to_bson;
    use super::*;
    use crate::host::generated::wasi::docstore::types::{ComparisonOp, ScalarValue};
    use crate::host::resource::FilterTree;

    fn temp_db() -> DocStoreDefault {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "omnia-docstore-test-{}-{n}.polodb",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("time")
                .as_nanos()
        ));
        let db = Database::open_path(path.to_string_lossy().as_ref()).expect("open temp db");
        DocStoreDefault { db: Arc::new(db) }
    }

    fn insert_doc(ctx: &DocStoreDefault, collection: &str, id: &str, val: &serde_json::Value) {
        let col = ctx.db.collection::<bson::Document>(collection);
        let mut bson_doc = bson::to_document(&val).expect("to bson");
        bson_doc.insert("_id", id);
        col.insert_one(bson_doc).expect("insert");
    }

    fn query_with_filter(
        ctx: &DocStoreDefault, collection: &str, filter: &FilterTree,
    ) -> Vec<bson::Document> {
        bson_filter::validate(filter).expect("filter validation");
        let col = ctx.db.collection::<bson::Document>(collection);
        let bson_filter = to_bson(filter);
        let cursor = col.find(bson_filter).run().expect("find");
        cursor.collect::<Result<Vec<_>, _>>().expect("collect")
    }

    // Insert + get round-trips are covered end-to-end by the seam test
    // (`tests/seam.rs`), which drives the same `DocStoreDefault::insert`/`get`
    // through the guest. What remains here is filter translation/validation —
    // pure logic with no seam equivalent.
    #[test]
    fn and_eq_int_with_is_not_null() {
        let ctx = temp_db();
        insert_doc(&ctx, "s", "s1", &json!({"wb": 1, "zone_id": "z1"}));
        insert_doc(&ctx, "s", "s2", &json!({"wb": 1, "zone_id": "z2"}));
        insert_doc(&ctx, "s", "s3", &json!({"wb": 0, "zone_id": "z3"}));
        insert_doc(&ctx, "s", "s4", &json!({"wb": 1, "zone_id": null}));

        let filter = FilterTree::And(vec![
            FilterTree::Compare {
                field: "wb".to_string(),
                op: ComparisonOp::Eq,
                value: ScalarValue::Int32(1),
            },
            FilterTree::IsNotNull("zone_id".to_string()),
        ]);
        let results = query_with_filter(&ctx, "s", &filter);
        assert_eq!(results.len(), 2, "wb=1 AND zone_id IS NOT NULL");
    }

    // Rejected: the default in-memory store does not support `starts-with`.
    #[test]
    fn starts_with() {
        let filter = FilterTree::StartsWith {
            field: "name".to_string(),
            pattern: "Northern".to_string(),
        };
        let err = bson_filter::validate(&filter).unwrap_err();
        assert!(
            err.to_string().contains("not supported"),
            "expected unsupported error, got: {err}"
        );
    }

    // Rejected: `$`-prefixed field names are a query-injection vector.
    #[test]
    fn dollar_prefixed_field() {
        let filter = FilterTree::Compare {
            field: "$where".to_string(),
            op: ComparisonOp::Eq,
            value: ScalarValue::Str("1".to_string()),
        };
        let err = bson_filter::validate(&filter).unwrap_err();
        assert!(
            err.to_string().contains("must not start with '$'"),
            "expected $ rejection, got: {err}"
        );
    }
}
