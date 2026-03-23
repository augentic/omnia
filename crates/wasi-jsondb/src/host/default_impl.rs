//! Default PoloDB-backed implementation of [`WasiJsonDbCtx`](crate::host::WasiJsonDbCtx).

#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_sign_loss)]
#![allow(missing_docs)]

use std::sync::Arc;

use anyhow::{Context, Result};
use futures::FutureExt;
use omnia::{Backend, FromEnv};
use polodb_core::bson::{self, doc};
use polodb_core::{CollectionT, Database};
use tracing::instrument;

use crate::host::bson_filter::to_bson;
use crate::host::generated::wasi::jsondb::types::{Document, QueryResult, SortField};
use crate::host::resource::FilterTree;
use crate::host::{FutureResult, QueryOpts, WasiJsonDbCtx};

/// Connection options for the embedded `PoloDB` file.
#[derive(Debug, Clone)]
pub struct ConnectOptions {
    /// Filesystem path to the `PoloDB` database file.
    pub database: String,
}

impl FromEnv for ConnectOptions {
    fn from_env() -> Result<Self> {
        let database = std::env::var("JSONDB_DATABASE").unwrap_or_else(|_| {
            std::env::temp_dir().join("omnia-jsondb.polodb").to_string_lossy().into_owned()
        });
        Ok(Self { database })
    }
}

/// Default [`WasiJsonDbCtx`] using `PoloDB`.
#[derive(Clone)]
pub struct JsonDbDefault {
    db: Arc<Database>,
}

impl std::fmt::Debug for JsonDbDefault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JsonDbDefault").field("db", &"<polodb Database>").finish()
    }
}

impl Backend for JsonDbDefault {
    type ConnectOptions = ConnectOptions;

    #[instrument]
    async fn connect_with(options: Self::ConnectOptions) -> Result<Self> {
        tracing::debug!("opening PoloDB at {}", options.database);
        let db = Database::open_path(&options.database).context("opening PoloDB database")?;
        Ok(Self { db: Arc::new(db) })
    }
}

impl WasiJsonDbCtx for JsonDbDefault {
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
                .map_err(|e| {
                    if is_duplicate_key_error(&e) {
                        anyhow::anyhow!("document id already exists")
                    } else {
                        anyhow::anyhow!("{e}")
                    }
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
            let col = db.collection::<bson::Document>(&collection);
            let bson_filter = filter.map_or_else(|| doc! {}, |f| to_bson(&f));

            let skip_u64 = parse_continuation(options.continuation.as_deref()).unwrap_or(0)
                + u64::from(options.offset.unwrap_or(0));

            let sort_doc = build_sort_document(&options.order_by);
            let limit = options.limit.map(u64::from);

            let mut find = col.find(bson_filter);
            if let Some(s) = sort_doc {
                find = find.sort(s);
            }
            find = find.skip(skip_u64);
            let fetch_limit = limit.map(|l| l.saturating_add(1));
            if let Some(l) = fetch_limit {
                find = find.limit(l);
            }

            let cursor = find.run().context("find query")?;
            let mut raw = Vec::new();
            for item in cursor {
                raw.push(item.context("cursor item")?);
            }
            let mut has_more = false;
            if let Some(lim) = limit
                && raw.len() > lim as usize
            {
                has_more = true;
                raw.truncate(lim as usize);
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

fn parse_continuation(continuation: Option<&str>) -> Option<u64> {
    continuation?.parse().ok()
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

fn is_duplicate_key_error(err: &polodb_core::Error) -> bool {
    format!("{err}").to_lowercase().contains("duplicate")
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::host::bson_filter::to_bson;
    use crate::host::generated::wasi::jsondb::types::{ComparisonOp, ScalarValue};
    use crate::host::resource::FilterTree;

    fn temp_db() -> JsonDbDefault {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "omnia-jsondb-test-{}-{n}.polodb",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("time")
                .as_nanos()
        ));
        let db = Database::open_path(path.to_string_lossy().as_ref()).expect("open temp db");
        JsonDbDefault { db: Arc::new(db) }
    }

    fn insert_doc(ctx: &JsonDbDefault, collection: &str, id: &str, val: serde_json::Value) {
        let col = ctx.db.collection::<bson::Document>(collection);
        let mut bson_doc = bson::to_document(&val).expect("to bson");
        bson_doc.insert("_id", id);
        col.insert_one(bson_doc).expect("insert");
    }

    fn query_with_filter(
        ctx: &JsonDbDefault, collection: &str, filter: FilterTree,
    ) -> Vec<bson::Document> {
        let col = ctx.db.collection::<bson::Document>(collection);
        let bson_filter = to_bson(&filter);
        let cursor = col.find(bson_filter).run().expect("find");
        cursor.collect::<Result<Vec<_>, _>>().expect("collect")
    }

    #[tokio::test]
    async fn roundtrip_document() {
        let path = std::env::temp_dir().join(format!(
            "omnia-jsondb-test-{}.polodb",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("time")
                .as_nanos()
        ));
        let path = path.to_string_lossy().into_owned();
        let ctx =
            JsonDbDefault::connect_with(ConnectOptions { database: path }).await.expect("connect");

        let doc = Document {
            id: "a1".to_string(),
            data: serde_json::to_vec(&json!({ "name": "x" })).expect("json"),
        };

        ctx.insert("c".to_string(), doc.clone()).await.expect("insert");
        let got = ctx.get("c".to_string(), "a1".to_string()).await.expect("get");
        assert_eq!(got.unwrap().data, doc.data);
    }

    #[test]
    fn not_equal_via_filter_tree() {
        let ctx = temp_db();
        insert_doc(&ctx, "r", "r1", json!({"route_type": 3}));
        insert_doc(&ctx, "r", "r2", json!({"route_type": 4}));
        insert_doc(&ctx, "r", "r3", json!({"route_type": 2}));

        let filter = FilterTree::Not(Box::new(FilterTree::Compare {
            field: "route_type".to_string(),
            op: ComparisonOp::Eq,
            value: ScalarValue::Int32(4),
        }));
        let results = query_with_filter(&ctx, "r", filter);
        assert_eq!(results.len(), 2, "Not(Eq(4)) should exclude route_type=4");
    }

    #[test]
    fn is_not_null_via_filter_tree() {
        let ctx = temp_db();
        insert_doc(&ctx, "s", "s1", json!({"zone_id": "z1"}));
        insert_doc(&ctx, "s", "s2", json!({"zone_id": null}));
        insert_doc(&ctx, "s", "s3", json!({"zone_id": "z2"}));

        let filter = FilterTree::IsNotNull("zone_id".to_string());
        let results = query_with_filter(&ctx, "s", filter);
        assert_eq!(results.len(), 2, "IsNotNull should exclude null zone_id");
    }

    #[test]
    fn contains_via_filter_tree() {
        let ctx = temp_db();
        insert_doc(&ctx, "s", "s1", json!({"name": "Albany Station"}));
        insert_doc(&ctx, "s", "s2", json!({"name": "Newmarket Station"}));
        insert_doc(&ctx, "s", "s3", json!({"name": "Ponsonby Rd"}));

        let filter = FilterTree::Contains {
            field: "name".to_string(),
            pattern: "Station".to_string(),
        };
        let results = query_with_filter(&ctx, "s", filter);
        assert_eq!(results.len(), 2, "Contains('Station') should match 2 stops");
    }

    #[test]
    fn and_eq_int_with_is_not_null() {
        let ctx = temp_db();
        insert_doc(&ctx, "s", "s1", json!({"wb": 1, "zone_id": "z1"}));
        insert_doc(&ctx, "s", "s2", json!({"wb": 1, "zone_id": "z2"}));
        insert_doc(&ctx, "s", "s3", json!({"wb": 0, "zone_id": "z3"}));
        insert_doc(&ctx, "s", "s4", json!({"wb": 1, "zone_id": null}));

        let filter = FilterTree::And(vec![
            FilterTree::Compare {
                field: "wb".to_string(),
                op: ComparisonOp::Eq,
                value: ScalarValue::Int32(1),
            },
            FilterTree::IsNotNull("zone_id".to_string()),
        ]);
        let results = query_with_filter(&ctx, "s", filter);
        assert_eq!(results.len(), 2, "wb=1 AND zone_id IS NOT NULL");
    }

    #[test]
    fn not_in_list_via_filter_tree() {
        let ctx = temp_db();
        insert_doc(&ctx, "r", "r1", json!({"v": 1}));
        insert_doc(&ctx, "r", "r2", json!({"v": 2}));
        insert_doc(&ctx, "r", "r3", json!({"v": 3}));

        let filter = FilterTree::NotInList {
            field: "v".to_string(),
            values: vec![ScalarValue::Int32(1), ScalarValue::Int32(2)],
        };
        let results = query_with_filter(&ctx, "r", filter);
        assert_eq!(results.len(), 1, "NotInList([1,2]) should return only v=3");
    }

    #[test]
    fn starts_with_via_filter_tree() {
        let ctx = temp_db();
        insert_doc(&ctx, "sw", "s1", json!({"name": "Northern Express"}));
        insert_doc(&ctx, "sw", "s2", json!({"name": "Eastern Line"}));
        insert_doc(&ctx, "sw", "s3", json!({"name": "Inner Link"}));

        let filter = FilterTree::StartsWith {
            field: "name".to_string(),
            pattern: "Northern".to_string(),
        };
        let results = query_with_filter(&ctx, "sw", filter);
        assert_eq!(results.len(), 1, "StartsWith('Northern') should match 1");
    }
}
