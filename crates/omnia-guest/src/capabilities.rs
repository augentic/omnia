//! Provider capabilities: the host services a guest's provider can call.
//!
//! Each capability is a trait whose methods carry WASI-backed default bodies on
//! `wasm32` (delegating to the matching `omnia-wasi-*` binding) and bare
//! signatures off `wasm32`, so hosts and tests can supply their own.
//!
//! Every capability is also implemented for `Arc<T>`, `&T`, and `Box<T>`
//! where `T` implements it, so a provider may hold a capability behind a
//! pointer and a `P: Capability` bound accepts one.

// The pointer impls forward on both targets: an empty impl would hand a
// wrapped provider the WASI defaults in place of its own overrides.
macro_rules! forward_pointers {
    ($trait:ident { $($body:tt)* }) => {
        impl<P: $trait + ?Sized> $trait for ::std::sync::Arc<P> { $($body)* }
        impl<P: $trait + ?Sized> $trait for &P { $($body)* }
        impl<P: $trait + ?Sized> $trait for ::std::boxed::Box<P> { $($body)* }
    };
}

mod blob;
mod broadcast;
mod config;
#[cfg(feature = "orm")]
mod document;
mod http;
mod identity;
mod messaging;
pub mod model;
pub mod plugins;
mod state;
#[cfg(feature = "orm")]
mod table;

pub use blob::{BlobStore, BlobStoreExt, ContainerMetadata, ObjectMetadata};
pub use broadcast::Broadcast;
pub use config::Config;
#[cfg(feature = "orm")]
pub use document::DocumentStore;
pub use http::HttpRequest;
pub use identity::Identity;
pub use messaging::{Message, Publish};
// Generic model wire names stay scoped to the model capability.
pub use model::Model;
#[cfg(target_arch = "wasm32")]
pub use model::WasiModel;
// Loader request names stay scoped to the plugins capability.
pub use plugins::Plugins;
#[cfg(target_arch = "wasm32")]
pub use plugins::WasiPlugins;
pub use state::{CasError, StateStore};
#[cfg(feature = "orm")]
pub use table::TableStore;
