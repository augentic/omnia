//! The store and trivial doubles at the handler rung.

#![cfg(not(target_arch = "wasm32"))]

use std::future::Future;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::pin::pin;
use std::sync::Arc;
use std::task::{Context, Poll, Waker};
use std::thread;
use std::time::Duration;

use bytes::Bytes;
use http::{Method, Request, Response};
use omnia_sdk::capability::ErrorKind;
use omnia_sdk::docstore::{Document, Filter, Query, SortField};
use omnia_sdk::messaging::Message;
use omnia_sdk::sql::{Field, Row, Statement, Value};
use omnia_sdk::{
    BlobStore, DocumentDb, HttpClient, Identity, Publisher, SecretStore, SqlDb, StateStore,
    WebSockets,
};
use omnia_test::guest::{
    Broadcasted, FixedIdentity, MatchedHttp, Memory, MemoryDocs, ScriptedTables, Sink,
};

// every store double answers without yielding
fn now<F: Future>(future: F) -> F::Output {
    let mut future = pin!(future);
    match future.as_mut().poll(&mut Context::from_waker(Waker::noop())) {
        Poll::Ready(output) => output,
        Poll::Pending => panic!("a store double must not yield"),
    }
}

fn doc(id: &str, json: &str) -> Document {
    Document::new(id, json.as_bytes())
}

fn names<P: BlobStore>(
    store: &P, container: &str, prefix: &str, after: Option<&str>,
) -> Vec<String> {
    let page = now(store.list(container, prefix, after)).expect("list");
    assert_eq!(page.next, None, "a listing answers one page");
    page.items.into_iter().map(|object| object.name).collect()
}

// -- Memory: state -----------------------------------------------------------

#[test]
fn state() {
    let memory = Memory::default().bucket("sessions");
    let sessions = memory.named("sessions");
    now(StateStore::set(&memory, "k", b"v1", None)).expect("set");
    now(StateStore::set(&memory, "k", b"v2", Some(Duration::from_secs(30)))).expect("set");
    now(StateStore::set(&sessions, "k", b"s", None)).expect("set");
    assert_eq!(memory.state("k"), Some(b"v2".to_vec()));
    assert_eq!(sessions.state("k"), Some(b"s".to_vec()));

    now(StateStore::delete(&memory, "k")).expect("delete");
    now(StateStore::delete(&sessions, "k")).expect("delete");
    assert!(memory.is_empty());
    let (state, _) = memory.snapshot();
    assert_eq!(state.keys().collect::<Vec<_>>(), ["default", "sessions"]);
}

#[test]
fn cas_race() {
    const WRITERS: usize = 8;
    const ROUNDS: u64 = 200;

    let memory = Memory::default();
    let handles: Vec<_> = (0..WRITERS)
        .map(|_| {
            let memory = memory.clone();
            thread::spawn(move || {
                for _ in 0..ROUNDS {
                    loop {
                        let current = memory.state("counter");
                        let value: u64 = current.as_deref().map_or(0, |bytes| {
                            std::str::from_utf8(bytes).unwrap().parse().unwrap()
                        });
                        let next = (value + 1).to_string();
                        let swapped = now(memory.compare_and_swap(
                            "counter",
                            current.as_deref(),
                            next.as_bytes(),
                            None,
                        ));
                        if swapped.expect("store").is_ok() {
                            break;
                        }
                    }
                }
            })
        })
        .collect();
    for handle in handles {
        handle.join().expect("writer finishes");
    }

    let total = WRITERS as u64 * ROUNDS;
    assert_eq!(memory.state("counter"), Some(total.to_string().into_bytes()));
}

#[test]
fn increment_text() {
    let memory = Memory::default();
    memory.insert_state("text", b"nope");

    let error = now(memory.increment("text", 1, None)).expect_err("non-integer bytes fail");

    assert_eq!(error.kind(), ErrorKind::Invalid, "{error}");
    assert!(error.to_string().contains("8-byte"), "{error}");
}

// -- Memory: blobs -----------------------------------------------------------

#[test]
fn objects() {
    let memory = Memory::default();
    now(memory.create_container("c")).expect("create");
    assert!(now(memory.container_exists("c")).expect("exists"));
    now(memory.put("c", "b", b"bee")).expect("put");
    now(memory.put("c", "a", b"ay")).expect("put");
    let got = now(BlobStore::get(&memory, "c", "a")).expect("get");
    assert_eq!(got.as_deref(), Some(&b"ay"[..]));
    assert_eq!(now(BlobStore::get(&memory, "c", "missing")).expect("get"), None);
    assert!(now(BlobStore::exists(&memory, "c", "b")).expect("exists"));
    assert_eq!(names(&memory, "c", "", None), ["a", "b"]);
    now(BlobStore::delete(&memory, "c", "a")).expect("delete");
    assert_eq!(memory.objects("c"), ["b"]);
    now(memory.delete_many("c", &["b"])).expect("delete many");
    assert_eq!(memory.objects("c"), Vec::<String>::new());
    assert!(memory.has_container("c"));
    now(memory.delete_container("c")).expect("delete container");
    assert!(!memory.has_container("c"));
}

#[test]
fn listing() {
    let memory = Memory::default();
    for name in ["logs/2", "logs/1", "images/a", "logs/3"] {
        memory.insert_object("c", name, b"x");
    }

    assert_eq!(names(&memory, "c", "logs/", None), ["logs/1", "logs/2", "logs/3"]);
    assert_eq!(names(&memory, "c", "logs/", Some("logs/1")), ["logs/2", "logs/3"]);
    assert_eq!(names(&memory, "c", "", Some("images/a")), ["logs/1", "logs/2", "logs/3"]);
    assert_eq!(names(&memory, "missing", "", None), Vec::<String>::new());
}

#[test]
fn get_range() {
    let memory = Memory::default();
    memory.insert_object("c", "o", b"0123456789");
    let range = |range| now(memory.get_range("c", "o", range)).expect("range");

    assert_eq!(range(2..5).as_deref(), Some(&b"234"[..]));
    assert_eq!(range(7..u64::MAX).as_deref(), Some(&b"789"[..]));
    assert_eq!(range(5..100).as_deref(), Some(&b"56789"[..]));
    assert_eq!(range(4..4).as_deref(), Some(&b""[..]));
    assert_eq!(range(20..30).as_deref(), Some(&b""[..]));
    assert_eq!(now(memory.get_range("c", "missing", 0..1)).expect("range"), None);
}

#[test]
fn stat_timestamps() {
    let memory = Memory::default();
    now(memory.create_container("c")).expect("create");
    now(memory.put("c", "first", b"1")).expect("put");
    now(memory.put("c", "second", b"22")).expect("put");
    let container = now(memory.container_info("c")).expect("container info").expect("container");
    let first = now(memory.stat("c", "first")).expect("stat").expect("first");
    let second = now(memory.stat("c", "second")).expect("stat").expect("second");

    assert!(container.created_at < first.created_at && first.created_at < second.created_at);
    assert_eq!((second.size, second.container.as_str()), (2, "c"));
    assert_eq!(now(memory.stat("c", "missing")).expect("stat"), None);
    assert_eq!(now(memory.container_info("missing")).expect("container info"), None);
}

#[test]
fn copy_rename_clear() {
    let memory = Memory::default();
    memory.insert_object("src", "o", b"data");
    now(memory.copy("src", "o", "dst", "copy")).expect("copy");
    now(memory.rename("src", "o", "dst", "moved")).expect("rename");
    assert_eq!(memory.objects("src"), Vec::<String>::new());
    assert_eq!(memory.objects("dst"), ["copy", "moved"]);
    now(memory.rename("dst", "moved", "dst", "moved")).expect("rename onto itself");
    assert_eq!(memory.object("dst", "moved"), Some(b"data".to_vec()));

    now(memory.clear("dst")).expect("clear");
    assert!(memory.objects("dst").is_empty() && memory.has_container("dst"));
    let error = now(memory.copy("src", "gone", "dst", "x")).expect_err("no source");
    assert_eq!(error.kind(), ErrorKind::NotFound, "{error}");
}

// -- Memory: secrets ---------------------------------------------------------

#[test]
fn secrets() {
    let memory = Memory::default();
    for key in ["b", "a", "c"] {
        now(SecretStore::set(&memory, key, key.as_bytes())).expect("set");
    }
    now(SecretStore::delete(&memory, "c")).expect("delete");

    let got = now(SecretStore::get(&memory, "a")).expect("get");
    assert_eq!(got.as_deref(), Some(&b"a"[..]));
    assert!(now(SecretStore::exists(&memory, "b")).expect("exists"));
    assert!(!now(SecretStore::exists(&memory, "c")).expect("exists"));
    assert_eq!(now(memory.keys(None)).expect("keys").items, ["a", "b"]);
    assert_eq!(now(memory.keys(Some("a"))).expect("keys").items, ["b"]);
}

#[test]
fn lockers() {
    let memory = Memory::default();
    let other = SecretStore::named(&memory, "other");
    now(SecretStore::set(&memory, "k", b"default")).expect("set");
    now(SecretStore::set(&other, "k", b"other")).expect("set");

    assert_eq!(memory.secret("k"), Some(b"default".to_vec()));
    assert_eq!(other.secret("k"), Some(b"other".to_vec()));
    let empty = now(SecretStore::named(&memory, "empty").keys(None)).expect("keys");
    assert_eq!(empty.items, Vec::<String>::new());
}

// -- Pointer impls -----------------------------------------------------------

// handler-shaped: bounded on the capability, not on `Memory`
fn bump<P: StateStore>(store: &P, delta: i64) -> i64 {
    now(store.increment("visits", delta, None)).expect("increment")
}

#[test]
fn shared_handle() {
    let memory = Memory::default().bucket("sessions");
    let shared = Arc::new(memory.clone());
    assert_eq!(bump(&shared, 1), 1);
    assert_eq!(bump(&&memory, 1), 2);
    assert_eq!(bump(&Box::new(memory.clone()), 1), 3);
    assert_eq!(bump(&StateStore::named(&shared, "sessions"), 1), 1);
    assert_eq!(bump(&memory, 0), 3);
    assert!(now(BlobStore::exists(&Arc::new(memory), "c", "missing")).is_ok_and(|found| !found));
}

// -- MemoryDocs --------------------------------------------------------------

#[test]
fn documents() {
    let docs = MemoryDocs::default();
    assert_eq!(now(docs.insert("people", &doc("1", r#"{"name":"ann"}"#))).expect("insert"), Ok(()));
    now(docs.put("people", &doc("1", r#"{"name":"anne"}"#))).expect("put upserts");
    let stored = now(DocumentDb::get(&docs, "people", "1")).expect("get").expect("present");
    assert_eq!(&stored.data[..], br#"{"name":"anne"}"#);
    assert!(now(DocumentDb::delete(&docs, "people", "1")).expect("delete"));
    assert!(!now(DocumentDb::delete(&docs, "people", "1")).expect("delete"));

    let error = now(docs.put("people", &doc("2", "not json"))).expect_err("not JSON");
    assert_eq!(error.kind(), ErrorKind::Invalid, "{error}");
}

#[test]
fn queries() {
    let docs = MemoryDocs::default();
    for (id, age) in [("a", 30), ("b", 20), ("c", 40), ("d", 10)] {
        now(docs.put("people", &doc(id, &format!(r#"{{"age":{age}}}"#)))).expect("put");
    }
    let oldest = |after: Option<String>| Query {
        filter: Some(Filter::gte("age", 20)),
        order_by: vec![SortField {
            field: "age".into(),
            descending: true,
        }],
        limit: Some(2),
        after,
        ..Query::default()
    };

    let page = now(docs.query("people", &oldest(None))).expect("query");
    let ids: Vec<_> = page.items.iter().map(|d| d.id.as_str()).collect();
    assert_eq!(ids, ["c", "a"]);
    assert!(page.next.is_some());

    let rest = now(docs.query("people", &oldest(page.next))).expect("query");
    let ids: Vec<_> = rest.items.iter().map(|d| d.id.as_str()).collect();
    assert_eq!(ids, ["b"]);
    assert!(rest.next.is_none());
    assert_eq!(now(docs.documents("people")).expect("all").len(), 4);

    let malformed = Query {
        after: Some("not-a-number".into()),
        ..Query::default()
    };
    let error = now(docs.query("people", &malformed)).expect_err("malformed cursor");
    assert_eq!(error.kind(), ErrorKind::Invalid, "{error}");
}

#[test]
fn filter_and_sort() {
    let docs = MemoryDocs::default();
    for (id, body) in [
        (
            "a",
            r#"{"name":"Newmarket Station","route_type":2,"stop":{"zone":{"id":"z1"}},"updated":"2026-03-19T12:00:00Z","rank":1}"#,
        ),
        (
            "b",
            r#"{"name":"Britomart","route_type":3,"stop":{"zone":{"id":"z2"}},"updated":"2026-03-20T00:00:00Z","rank":1}"#,
        ),
        (
            "c",
            r#"{"name":"Northern Line","route_type":1,"updated":"2026-12-31T23:59:59Z","rank":"str"}"#,
        ),
    ] {
        now(docs.put("stops", &doc(id, body))).expect("put");
    }

    let query_ids = |filter: Filter| -> Vec<String> {
        let query = Query {
            filter: Some(filter),
            ..Query::default()
        };
        now(docs.query("stops", &query)).expect("query").items.into_iter().map(|d| d.id).collect()
    };

    assert_eq!(query_ids(Filter::eq("stop.zone.id", "z1")), ["a"]);
    assert_eq!(query_ids(Filter::ne("stop.zone.id", "z1")), ["b", "c"]);
    assert_eq!(query_ids(Filter::in_list("route_type", [2, 3])), ["a", "b"]);
    assert_eq!(query_ids(Filter::not_in_list("route_type", [1, 2])), ["b"]);
    assert_eq!(query_ids(Filter::is_null("stop.zone.id")), ["c"]);
    assert_eq!(query_ids(Filter::contains("name", "Station")), ["a"]);
    assert_eq!(query_ids(Filter::starts_with("name", "Northern")), ["c"]);
    assert_eq!(query_ids(Filter::ends_with("name", "Line")), ["c"]);
    assert_eq!(
        query_ids(Filter::or([
            Filter::starts_with("name", "Brit"),
            Filter::ends_with("name", "Line"),
        ])),
        ["b", "c"]
    );
    assert_eq!(query_ids(Filter::negate(Filter::eq("route_type", 1))), ["a", "b"]);
    assert_eq!(query_ids(Filter::on_date("updated", "2026-03-19").expect("valid date")), ["a"]);
    assert_eq!(query_ids(Filter::on_date("updated", "2026-03-20").expect("valid date")), ["b"]);
    assert_eq!(query_ids(Filter::on_date("updated", "2026-12-31").expect("month end")), ["c"]);
    Filter::on_date("updated", "2026-13-01").expect_err("invalid month");

    // Multi-field sort and mixed JSON type ordering (number < string).
    let sorted = Query {
        order_by: vec![
            SortField {
                field: "rank".into(),
                descending: false,
            },
            SortField {
                field: "route_type".into(),
                descending: true,
            },
        ],
        ..Query::default()
    };
    let sorted = now(docs.query("stops", &sorted)).expect("sorted query");
    let sorted_ids: Vec<_> = sorted.items.iter().map(|d| d.id.as_str()).collect();
    assert_eq!(sorted_ids, ["b", "a", "c"]);
}

// -- ScriptedTables ----------------------------------------------------------

#[test]
fn tables() {
    let row = Row {
        fields: vec![Field {
            name: "n".into(),
            value: Value::from(1_i32),
        }],
    };
    let tables = ScriptedTables::default()
        .on_query(|sql, _| sql.starts_with("SELECT"), vec![row])
        .on_exec(|sql, params| sql.starts_with("INSERT") && params.len() == 1, 1);

    let rows = now(tables.query("SELECT 1", &[])).expect("query");
    assert_eq!(rows.len(), 1);
    let affected =
        now(tables.named("orders").execute("INSERT", &[Value::from("x")])).expect("exec");
    assert_eq!(affected, 1);
    let batch = [
        Statement::new("INSERT", [Value::from("y")]),
        Statement::new("INSERT", [Value::from("z")]),
    ];
    assert_eq!(now(tables.batch(&batch)).expect("batch"), [1, 1]);

    let statements = tables.statements();
    assert_eq!(statements.len(), 4);
    assert_eq!(
        (statements[0].binding.as_str(), statements[0].sql.as_str()),
        ("default", "SELECT 1")
    );
    assert_eq!((statements[1].binding.as_str(), statements[1].sql.as_str()), ("orders", "INSERT"));
    assert_eq!(statements[3].params, [Value::from("z")]);
}

#[test]
fn unmatched_statement() {
    let tables = ScriptedTables::default();
    let result = catch_unwind(AssertUnwindSafe(|| {
        drop(tables.execute("DROP TABLE t", &[]));
    }));
    let payload = result.expect_err("unmatched execute panics");
    let text = payload.downcast_ref::<String>().cloned().unwrap_or_default();
    assert!(text.contains("DROP TABLE t"), "{text}");
}

// -- MatchedHttp -------------------------------------------------------------

#[test]
fn http() {
    let http = MatchedHttp::default()
        .on(Method::GET, "https://api.test/ping", Response::new(Bytes::from_static(b"pong")))
        .on_matching(
            Method::POST,
            "https://api.test/items",
            |body| body.starts_with(b"{"),
            Response::builder().status(201).body(Bytes::new()).expect("response"),
        );

    let ping = Request::get("https://api.test/ping").body(Bytes::new()).expect("request");
    assert_eq!(now(http.fetch(ping)).expect("fetch").into_body(), "pong");

    let create = Request::post("https://api.test/items")
        .header("content-type", "application/json")
        .body(Bytes::from_static(b"{\"a\":1}"))
        .expect("request");
    assert_eq!(now(http.fetch(create)).expect("fetch").status(), 201);

    let requests = http.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[1].text(), "{\"a\":1}");
    assert_eq!(requests[1].headers["content-type"], "application/json");
}

#[test]
fn unmatched_request() {
    let http = MatchedHttp::default();
    let request = Request::delete("https://api.test/x").body(Bytes::new()).expect("request");
    let result = catch_unwind(AssertUnwindSafe(|| drop(now(http.fetch(request)))));
    let payload = result.expect_err("unmatched request panics");
    let text = payload.downcast_ref::<String>().cloned().unwrap_or_default();
    assert!(text.contains("DELETE https://api.test/x"), "{text}");
}

// -- Sink, FixedIdentity -----------------------------------------------------

#[test]
fn sink() {
    let sink = Sink::default();
    now(sink.publish("orders", &Message::new(b"one"))).expect("publish");
    now(sink.send_to(&["s1", "s2"], b"hello")).expect("send to");
    now(sink.broadcast(b"everyone")).expect("broadcast");

    assert_eq!(sink.sent().len(), 1);
    assert_eq!(
        sink.broadcasts(),
        [
            Broadcasted {
                data: b"hello".to_vec(),
                sockets: Some(vec!["s1".to_owned(), "s2".to_owned()]),
            },
            Broadcasted {
                data: b"everyone".to_vec(),
                sockets: None,
            },
        ]
    );
}

#[test]
fn fixed_identity() {
    let identity = FixedIdentity::new("tok");

    let token = now(identity.token(&["scope"])).expect("token");

    assert_eq!((token.token.as_str(), token.expires_in), ("tok", Duration::from_secs(3600)));
}
