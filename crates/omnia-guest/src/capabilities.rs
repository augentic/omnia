//! Provider capabilities: the host services a guest's provider can call.
//!
//! Each capability is a trait whose methods carry WASI-backed default bodies on
//! `wasm32` (delegating to the matching `omnia-wasi-*` binding) and bare
//! signatures off `wasm32`, so hosts and tests can supply their own.

mod blob;
mod broadcast;
mod config;
mod document;
mod http;
mod identity;
mod messaging;
pub mod model;
mod state;
mod table;

pub use blob::{BlobStore, ContainerMetadata, ObjectMetadata};
pub use broadcast::Broadcast;
pub use config::Config;
pub use document::DocumentStore;
pub use http::HttpRequest;
pub use identity::Identity;
pub use messaging::{Message, Publish};
// `model`'s request/reply types stay module-scoped: `Message`, `Request`, and
// `Reply` would otherwise collide with `messaging` and `api` in the crate's
// flat re-exports.
pub use model::Model;
pub use state::StateStore;
pub use table::TableStore;
