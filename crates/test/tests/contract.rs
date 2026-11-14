//! The `omnia_test::contract` rows over the native doubles.

#![cfg(not(target_arch = "wasm32"))]

use std::time::Duration;

use omnia_test::contract::{blob, document, state};
use omnia_test::guest::{Memory, MemoryDocs};

const SCOPE: &str = "contract";

fn memory() -> Memory {
    Memory::default().bucket(state::SECOND)
}

// every `Memory` call is synchronous, so a blocking sleep is the store's clock
async fn wait(duration: Duration) {
    std::thread::sleep(duration);
}

#[tokio::test]
async fn state_persist() {
    state::persist(&memory(), wait).await;
}

#[tokio::test]
async fn state_set_ttl() {
    state::set_ttl(&memory(), wait).await;
}

#[tokio::test]
async fn state_zero_ttl() {
    state::zero_ttl(&memory()).await;
}

#[tokio::test]
async fn state_create_ttl() {
    state::create_ttl(&memory(), wait).await;
}

#[tokio::test]
async fn state_update_ttl() {
    state::update_ttl(&memory(), wait).await;
}

#[tokio::test]
async fn state_swap_expiring() {
    state::swap_expiring(&memory()).await;
}

#[tokio::test]
async fn state_conflict() {
    state::conflict(&memory()).await;
}

#[tokio::test]
async fn state_counter() {
    state::counter(&memory()).await;
}

#[tokio::test]
async fn state_named() {
    state::named(&memory()).await;
}

#[tokio::test]
async fn state_undeclared() {
    state::undeclared(&memory()).await;
}

#[tokio::test]
async fn state_delete_absent() {
    state::delete_absent(&memory()).await;
}

#[tokio::test]
async fn state_portable_key() {
    state::portable_key(&memory()).await;
}

#[tokio::test]
async fn state_malformed_json() {
    state::malformed_json(&memory()).await;
}

#[tokio::test]
async fn document_insert_exists() {
    document::insert_exists(&MemoryDocs::default()).await;
}

#[tokio::test]
async fn document_portable_query() {
    document::portable_query(&MemoryDocs::default()).await;
}

#[tokio::test]
async fn document_filter_limits() {
    document::filter_limits(&MemoryDocs::default()).await;
}

#[tokio::test]
async fn document_optional_query() {
    document::optional_query(&MemoryDocs::default()).await;
}

#[tokio::test]
async fn blob_objects() {
    blob::objects(&Memory::default(), SCOPE).await;
}

#[tokio::test]
async fn blob_absent() {
    blob::absent(&Memory::default(), SCOPE).await;
}

#[tokio::test]
async fn blob_delete_absent() {
    blob::delete_absent(&Memory::default(), SCOPE).await;
}

#[tokio::test]
async fn blob_uncreated() {
    blob::uncreated(&Memory::default(), SCOPE).await;
}

#[tokio::test]
async fn blob_containers() {
    blob::containers(&Memory::default(), SCOPE).await;
}

#[tokio::test]
async fn blob_range() {
    blob::range(&Memory::default(), SCOPE).await;
}

#[tokio::test]
async fn blob_listing() {
    blob::listing(&Memory::default(), SCOPE).await;
}

#[tokio::test]
async fn blob_copy_rename() {
    blob::copy_rename(&Memory::default(), SCOPE).await;
}

#[tokio::test]
async fn blob_clear() {
    blob::clear(&Memory::default(), SCOPE).await;
}
