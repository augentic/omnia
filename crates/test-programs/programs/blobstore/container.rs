//! Raw bindings: what a `container` handle answers over the in-memory
//! default, in the container the host seeds.
//!
//! - A delimited listing folds each name below a `/` into a common prefix,
//!   and pages of one walk objects and prefixes in name order.
//! - A range clamps to the object, and its `content` describes the whole
//!   object.
//! - Instants are milliseconds since the Unix epoch.
//! - A `put` whose `finished` is dropped answers the guest's own error, and
//!   one whose stream is shorter than its `size` is `invalid`.

#![cfg(target_arch = "wasm32")]

use omnia_wasi_blobstore::omnia::container::{ByteRange, Container, Content, Listing};
use omnia_wasi_blobstore::omnia::store;
use omnia_wasi_blobstore::omnia::types::{Attributes, Conflict, Error, ObjectEntry, Stored};
use omnia_wasi_blobstore::{wit_future, wit_stream};

omnia_sdk::api::command::export!(scenario);

// the starts of 2020 and of 2100, in milliseconds since the Unix epoch
const YEAR_2020_MS: u64 = 1_577_836_800_000;
const YEAR_2100_MS: u64 = 4_102_444_800_000;

// a listing still running after this many pages never ends
const MAX_PAGES: usize = 8;

async fn scenario() {
    let objects = store::open("objects".to_owned()).await.expect("open the seeded container");

    // writes that declare their size
    for (name, data) in [
        ("a/1", &b"a1"[..]),
        ("a/2", b"a2"),
        ("b", b"b"),
        ("c/d/e", b"cde"),
        ("c/x", b"cx"),
        ("digits", b"0123456789"),
    ] {
        let size = Some(data.len() as u64);
        let written = put(&objects, name, size, data.to_vec(), Some(Ok(()))).await;
        written.unwrap_or_else(|err| panic!("put `{name}`: {err:?}")).expect("unconditional");
    }

    // a delimited listing folds deeper names into prefixes
    let top = objects.list(String::new(), Listing::Delimited, None, None).await.expect("list");
    assert_eq!(names(&top.objects), ["b", "digits"]);
    assert_eq!(top.common_prefixes, ["a/", "c/"]);
    let nested = objects.list("c/".to_owned(), Listing::Delimited, None, None).await;
    let nested = nested.expect("list");
    assert_eq!(names(&nested.objects), ["c/x"]);
    assert_eq!(nested.common_prefixes, ["c/d/"]);
    assert_eq!(walk(&objects).await, ["a/", "b", "c/", "digits"]);

    // a range clamps to the object
    for (range, expected) in [
        (ByteRange::Offset(7), &b"789"[..]),
        (ByteRange::Offset(20), b""),
        (ByteRange::Suffix(3), b"789"),
        (ByteRange::Suffix(20), b"0123456789"),
        (ByteRange::Span((2, 5)), b"234"),
        (ByteRange::Span((5, 2)), b""),
    ] {
        let shown = format!("{range:?}");
        let content = objects.get("digits".to_owned(), Some(range)).await.expect("get");
        let Content {
            metadata,
            data,
            finished,
        } = content.expect("a stored object");
        assert_eq!(metadata.size, 10, "{shown} describes the whole object");
        let bytes = data.collect().await;
        finished.await.expect("every selected byte arrives");
        assert_eq!(bytes, expected, "{shown}");
    }

    // instants are milliseconds
    let stat = objects.stat("digits".to_owned()).await.expect("stat").expect("a stored object");
    assert!((YEAR_2020_MS..YEAR_2100_MS).contains(&stat.modified_at), "{}", stat.modified_at);
    let info = objects.info().await.expect("info");
    let created = info.created_at.expect("the default records when it created a container");
    assert!((YEAR_2020_MS..YEAR_2100_MS).contains(&created), "{created}");

    // writes the guest abandons or cuts short
    let abandoned = put(&objects, "abandoned", None, b"abc".to_vec(), None).await;
    assert!(
        matches!(&abandoned, Err(Error::Other(detail)) if detail == "abandoned"),
        "{abandoned:?}"
    );
    let short = put(&objects, "short", Some(10), b"abc".to_vec(), Some(Ok(()))).await;
    assert!(matches!(short, Err(Error::Invalid(_))), "{short:?}");
}

// Write `data` as the object `name`, then resolve `finished` with `verdict`,
// or drop it unresolved when there is none.
async fn put(
    container: &Container, name: &str, size: Option<u64>, data: Vec<u8>,
    verdict: Option<Result<(), Error>>,
) -> Result<Result<Stored, Conflict>, Error> {
    let (mut writer, reader) = wit_stream::new();
    let (finish, finished) = wit_future::new(|| Err(Error::Other("abandoned".to_owned())));
    let reply = container.put(name, size, reader, finished, &plain(), None);
    writer.write_all(data).await;
    drop(writer);
    match verdict {
        Some(verdict) => drop(finish.write(verdict).await),
        None => drop(finish),
    }
    reply.await
}

// the names and prefixes pages of one hold, in the order they hold them
async fn walk(container: &Container) -> Vec<String> {
    let (mut walked, mut after) = (Vec::new(), None);
    for _ in 0..MAX_PAGES {
        let page = container.list(String::new(), Listing::Delimited, after, Some(1)).await;
        let page = page.expect("list");
        assert_eq!(page.objects.len() + page.common_prefixes.len(), 1, "a page of one");
        walked.extend(names(&page.objects));
        walked.extend(page.common_prefixes);
        match page.next {
            Some(next) => after = Some(next),
            None => return walked,
        }
    }
    panic!("the listing did not end within {MAX_PAGES} pages");
}

fn names(objects: &[ObjectEntry]) -> Vec<String> {
    objects.iter().map(|object| object.name.clone()).collect()
}

fn plain() -> Attributes {
    Attributes {
        content_type: None,
        content_encoding: None,
        content_language: None,
        content_disposition: None,
        cache_control: None,
        metadata: Vec::new(),
    }
}
