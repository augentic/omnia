//! Provider capabilities: the host services a guest's provider can call.
//!
//! Each capability is a trait whose methods carry WASI-backed default bodies on
//! `wasm32` (delegating to the matching `omnia-wasi-*` binding) and bare
//! signatures off `wasm32`, so hosts and tests can supply their own.
//!
//! Every capability is also implemented for `Arc<T>`, `&T`, and `Box<T>`
//! where `T` implements it, so a provider may hold a capability behind a
//! pointer and a `P: Capability` bound accepts one.

/// Internal helper emitted by `#[derive(omnia_sdk::Provider)]` and pointer
/// blanket impls so capability method signatures live in one place.
#[doc(hidden)]
#[macro_export]
macro_rules! __delegate {
    (@methods BlobStore; $this:ident => $($target:tt)+) => {
        fn get(
            &$this, container: &str, name: &str,
        ) -> impl ::std::future::Future<Output = $crate::anyhow::Result<::core::option::Option<::std::vec::Vec<u8>>>> + ::core::marker::Send {
            $crate::BlobStore::get(&$($target)+, container, name)
        }

        fn put(
            &$this, container: &str, name: &str, data: &[u8],
        ) -> impl ::std::future::Future<Output = $crate::anyhow::Result<()>> + ::core::marker::Send {
            $crate::BlobStore::put(&$($target)+, container, name, data)
        }

        fn delete(
            &$this, container: &str, name: &str,
        ) -> impl ::std::future::Future<Output = $crate::anyhow::Result<()>> + ::core::marker::Send {
            $crate::BlobStore::delete(&$($target)+, container, name)
        }

        fn list(
            &$this, container: &str,
        ) -> impl ::std::future::Future<Output = $crate::anyhow::Result<::std::vec::Vec<::std::string::String>>> + ::core::marker::Send {
            $crate::BlobStore::list(&$($target)+, container)
        }

        fn get_range(
            &$this, container: &str, name: &str, start: u64, end: u64,
        ) -> impl ::std::future::Future<Output = $crate::anyhow::Result<::std::vec::Vec<u8>>> + ::core::marker::Send {
            $crate::BlobStore::get_range(&$($target)+, container, name, start, end)
        }

        fn object_info(
            &$this, container: &str, name: &str,
        ) -> impl ::std::future::Future<Output = $crate::anyhow::Result<$crate::ObjectMetadata>> + ::core::marker::Send {
            $crate::BlobStore::object_info(&$($target)+, container, name)
        }

        fn create_container(
            &$this, name: &str,
        ) -> impl ::std::future::Future<Output = $crate::anyhow::Result<()>> + ::core::marker::Send {
            $crate::BlobStore::create_container(&$($target)+, name)
        }

        fn delete_container(
            &$this, name: &str,
        ) -> impl ::std::future::Future<Output = $crate::anyhow::Result<()>> + ::core::marker::Send {
            $crate::BlobStore::delete_container(&$($target)+, name)
        }

        fn container_exists(
            &$this, name: &str,
        ) -> impl ::std::future::Future<Output = $crate::anyhow::Result<bool>> + ::core::marker::Send {
            $crate::BlobStore::container_exists(&$($target)+, name)
        }

        fn container_info(
            &$this, container: &str,
        ) -> impl ::std::future::Future<Output = $crate::anyhow::Result<$crate::ContainerMetadata>> + ::core::marker::Send {
            $crate::BlobStore::container_info(&$($target)+, container)
        }
    };
    (@methods Broadcast; $this:ident => $($target:tt)+) => {
        fn send(
            &$this, name: &str, data: &[u8], sockets: ::core::option::Option<::std::vec::Vec<::std::string::String>>,
        ) -> impl ::std::future::Future<Output = $crate::anyhow::Result<()>> + ::core::marker::Send {
            $crate::Broadcast::send(&$($target)+, name, data, sockets)
        }
    };
    (@methods Config; $this:ident => $($target:tt)+) => {
        fn get(
            &$this, key: &str,
        ) -> impl ::std::future::Future<Output = $crate::anyhow::Result<::std::string::String>> + ::core::marker::Send {
            $crate::Config::get(&$($target)+, key)
        }
    };
    (@methods DocumentStore; $this:ident => $($target:tt)+) => {
        fn get(
            &$this, store: &str, id: &str,
        ) -> impl ::std::future::Future<Output = $crate::anyhow::Result<::core::option::Option<$crate::docstore::Document>>> + ::core::marker::Send {
            $crate::DocumentStore::get(&$($target)+, store, id)
        }

        fn insert(
            &$this, store: &str, doc: &$crate::docstore::Document,
        ) -> impl ::std::future::Future<Output = $crate::anyhow::Result<()>> + ::core::marker::Send {
            $crate::DocumentStore::insert(&$($target)+, store, doc)
        }

        fn put(
            &$this, store: &str, doc: &$crate::docstore::Document,
        ) -> impl ::std::future::Future<Output = $crate::anyhow::Result<()>> + ::core::marker::Send {
            $crate::DocumentStore::put(&$($target)+, store, doc)
        }

        fn delete(
            &$this, store: &str, id: &str,
        ) -> impl ::std::future::Future<Output = $crate::anyhow::Result<bool>> + ::core::marker::Send {
            $crate::DocumentStore::delete(&$($target)+, store, id)
        }

        fn query(
            &$this, store: &str, options: $crate::docstore::QueryOptions,
        ) -> impl ::std::future::Future<Output = $crate::anyhow::Result<$crate::docstore::QueryResult>> + ::core::marker::Send {
            $crate::DocumentStore::query(&$($target)+, store, options)
        }
    };
    (@methods HttpRequest; $this:ident => $($target:tt)+) => {
        fn fetch<T>(
            &$this, request: $crate::http::Request<T>,
        ) -> impl ::std::future::Future<Output = $crate::anyhow::Result<$crate::http::Response<$crate::bytes::Bytes>>> + ::core::marker::Send
        where
            T: $crate::http_body::Body + ::core::any::Any + ::core::marker::Send,
            T::Data: ::core::convert::Into<::std::vec::Vec<u8>>,
            T::Error: ::core::convert::Into<::std::boxed::Box<dyn ::core::error::Error + ::core::marker::Send + ::core::marker::Sync + 'static>>,
        {
            $crate::HttpRequest::fetch(&$($target)+, request)
        }
    };
    (@methods Identity; $this:ident => $($target:tt)+) => {
        fn access_token(
            &$this, identity: ::std::string::String,
        ) -> impl ::std::future::Future<Output = $crate::anyhow::Result<::std::string::String>> + ::core::marker::Send {
            $crate::Identity::access_token(&$($target)+, identity)
        }
    };
    (@methods Model; $this:ident => $($target:tt)+) => {
        fn complete(
            &$this, request: $crate::model::Request,
        ) -> impl ::std::future::Future<Output = ::core::result::Result<$crate::model::Reply, $crate::model::Error>> + ::core::marker::Send {
            $crate::Model::complete(&$($target)+, request)
        }

        fn complete_with<H, F>(
            &$this, request: $crate::model::Request, handler: H,
        ) -> impl ::std::future::Future<Output = ::core::result::Result<$crate::model::Reply, $crate::model::Error>> + ::core::marker::Send
        where
            H: ::core::ops::FnMut($crate::model::ToolCall) -> F + ::core::marker::Send,
            F: ::std::future::Future<Output = ::core::result::Result<::std::string::String, ::std::string::String>> + ::core::marker::Send,
        {
            $crate::Model::complete_with(&$($target)+, request, handler)
        }
    };
    (@methods Plugins; $this:ident => $($target:tt)+) => {
        fn load(
            &$this, from: &$crate::plugins::Location, digest: ::core::option::Option<&$crate::plugins::Digest>,
        ) -> impl ::std::future::Future<Output = ::core::result::Result<$crate::plugins::Plugin, $crate::plugins::Error>> + ::core::marker::Send {
            $crate::Plugins::load(&$($target)+, from, digest)
        }
    };
    (@methods Publish; $this:ident => $($target:tt)+) => {
        fn send(
            &$this, topic: &str, message: &$crate::Message,
        ) -> impl ::std::future::Future<Output = $crate::anyhow::Result<()>> + ::core::marker::Send {
            $crate::Publish::send(&$($target)+, topic, message)
        }
    };
    (@methods StateStore; $this:ident => $($target:tt)+) => {
        fn get(
            &$this, key: &str,
        ) -> impl ::std::future::Future<Output = $crate::anyhow::Result<::core::option::Option<::std::vec::Vec<u8>>>> + ::core::marker::Send {
            $crate::StateStore::get(&$($target)+, key)
        }

        fn set(
            &$this, key: &str, value: &[u8], ttl_secs: ::core::option::Option<u64>,
        ) -> impl ::std::future::Future<Output = $crate::anyhow::Result<::core::option::Option<::std::vec::Vec<u8>>>> + ::core::marker::Send {
            $crate::StateStore::set(&$($target)+, key, value, ttl_secs)
        }

        fn delete(
            &$this, key: &str,
        ) -> impl ::std::future::Future<Output = $crate::anyhow::Result<()>> + ::core::marker::Send {
            $crate::StateStore::delete(&$($target)+, key)
        }

        fn cas(
            &$this, key: &str, expected: ::core::option::Option<&[u8]>, value: &[u8],
        ) -> impl ::std::future::Future<Output = ::core::result::Result<(), $crate::CasError>> + ::core::marker::Send {
            $crate::StateStore::cas(&$($target)+, key, expected, value)
        }

        fn increment(
            &$this, key: &str, delta: i64,
        ) -> impl ::std::future::Future<Output = $crate::anyhow::Result<i64>> + ::core::marker::Send {
            $crate::StateStore::increment(&$($target)+, key, delta)
        }
    };
    (@methods TableStore; $this:ident => $($target:tt)+) => {
        fn query(
            &$this, conn_name: ::std::string::String, query: ::std::string::String, params: ::std::vec::Vec<$crate::sql::DataType>,
        ) -> impl ::std::future::Future<Output = $crate::anyhow::Result<::std::vec::Vec<$crate::sql::Row>>> + ::core::marker::Send {
            $crate::TableStore::query(&$($target)+, conn_name, query, params)
        }

        fn exec(
            &$this, conn_name: ::std::string::String, query: ::std::string::String, params: ::std::vec::Vec<$crate::sql::DataType>,
        ) -> impl ::std::future::Future<Output = $crate::anyhow::Result<u32>> + ::core::marker::Send {
            $crate::TableStore::exec(&$($target)+, conn_name, query, params)
        }
    };
    (@methods Vcs; $this:ident => $($target:tt)+) => {
        fn resolve(
            &$this, repo: &str, revision: &str,
        ) -> impl ::std::future::Future<Output = ::core::result::Result<::std::string::String, $crate::vcs::Error>> + ::core::marker::Send {
            $crate::Vcs::resolve(&$($target)+, repo, revision)
        }

        fn descends(
            &$this, repo: &str, ancestor: &str, descendant: &str,
        ) -> impl ::std::future::Future<Output = ::core::result::Result<bool, $crate::vcs::Error>> + ::core::marker::Send {
            $crate::Vcs::descends(&$($target)+, repo, ancestor, descendant)
        }

        fn head(
            &$this, at: &str,
        ) -> impl ::std::future::Future<Output = ::core::result::Result<::std::string::String, $crate::vcs::Error>> + ::core::marker::Send {
            $crate::Vcs::head(&$($target)+, at)
        }

        fn commit(
            &$this, at: &str, message: &str,
        ) -> impl ::std::future::Future<Output = ::core::result::Result<::core::option::Option<::std::string::String>, $crate::vcs::Error>> + ::core::marker::Send {
            $crate::Vcs::commit(&$($target)+, at, message)
        }

        fn merge(
            &$this, at: &str, revision: &str, message: &str, policy: &[$crate::vcs::Rule],
        ) -> impl ::std::future::Future<Output = ::core::result::Result<$crate::vcs::Merged, $crate::vcs::Error>> + ::core::marker::Send {
            $crate::Vcs::merge(&$($target)+, at, revision, message, policy)
        }

        fn log(
            &$this, repo: &str, revision: &str, base: &str,
        ) -> impl ::std::future::Future<Output = ::core::result::Result<::std::vec::Vec<$crate::vcs::Entry>, $crate::vcs::Error>> + ::core::marker::Send {
            $crate::Vcs::log(&$($target)+, repo, revision, base)
        }

        fn init(
            &$this, at: &str,
        ) -> impl ::std::future::Future<Output = ::core::result::Result<(), $crate::vcs::Error>> + ::core::marker::Send {
            $crate::Vcs::init(&$($target)+, at)
        }

        fn add(
            &$this, repo: &str, at: &str, revision: &str,
        ) -> impl ::std::future::Future<Output = ::core::result::Result<(), $crate::vcs::Error>> + ::core::marker::Send {
            $crate::Vcs::add(&$($target)+, repo, at, revision)
        }

        fn remove(
            &$this, at: &str,
        ) -> impl ::std::future::Future<Output = ::core::result::Result<(), $crate::vcs::Error>> + ::core::marker::Send {
            $crate::Vcs::remove(&$($target)+, at)
        }

        fn pending(
            &$this, at: &str,
        ) -> impl ::std::future::Future<Output = ::core::result::Result<::std::vec::Vec<$crate::vcs::Change>, $crate::vcs::Error>> + ::core::marker::Send {
            $crate::Vcs::pending(&$($target)+, at)
        }

        fn clone_repo(
            &$this, url: &str, at: &str, options: $crate::vcs::CloneOptions,
        ) -> impl ::std::future::Future<Output = ::core::result::Result<(), $crate::vcs::Error>> + ::core::marker::Send {
            $crate::Vcs::clone_repo(&$($target)+, url, at, options)
        }

        fn fetch(
            &$this, repo: &str, remote: &str,
        ) -> impl ::std::future::Future<Output = ::core::result::Result<(), $crate::vcs::Error>> + ::core::marker::Send {
            $crate::Vcs::fetch(&$($target)+, repo, remote)
        }

        fn label(
            &$this, repo: &str, name: &str, revision: &str,
        ) -> impl ::std::future::Future<Output = ::core::result::Result<(), $crate::vcs::Error>> + ::core::marker::Send {
            $crate::Vcs::label(&$($target)+, repo, name, revision)
        }

        fn labelled(
            &$this, repo: &str, name: &str,
        ) -> impl ::std::future::Future<Output = ::core::result::Result<::std::string::String, $crate::vcs::Error>> + ::core::marker::Send {
            $crate::Vcs::labelled(&$($target)+, repo, name)
        }

        fn fetched(
            &$this, repo: &str, remote: &str, name: &str,
        ) -> impl ::std::future::Future<Output = ::core::result::Result<::std::string::String, $crate::vcs::Error>> + ::core::marker::Send {
            $crate::Vcs::fetched(&$($target)+, repo, remote, name)
        }

        fn push(
            &$this, repo: &str, remote: &str, label: &str,
        ) -> impl ::std::future::Future<Output = ::core::result::Result<(), $crate::vcs::Error>> + ::core::marker::Send {
            $crate::Vcs::push(&$($target)+, repo, remote, label)
        }
    };
    (@field $trait:ident; $this:ident, $field:ident) => {
        $crate::__delegate!(@methods $trait; $this => $this.$field);
    };
    (@deref $trait:ident) => {
        impl<P: $crate::$trait + ?Sized> $crate::$trait for ::std::sync::Arc<P> {
            $crate::__delegate!(@methods $trait; self => **self);
        }
        impl<P: $crate::$trait + ?Sized> $crate::$trait for &P {
            $crate::__delegate!(@methods $trait; self => **self);
        }
        impl<P: $crate::$trait + ?Sized> $crate::$trait for ::std::boxed::Box<P> {
            $crate::__delegate!(@methods $trait; self => **self);
        }
    };
}

// The pointer impls delegate on both targets: an empty impl would hand a
// wrapped provider the WASI defaults in place of its own overrides.
macro_rules! delegate_deref {
    ($trait:ident $($_body:tt)*) => {
        $crate::__delegate!(@deref $trait);
    };
}

mod blob;
mod broadcast;
mod config;
#[cfg(feature = "docstore")]
mod document;
mod http;
mod identity;
mod lend;
mod messaging;
pub mod model;
pub mod plugins;
mod state;
#[cfg(feature = "sql")]
mod table;
pub mod vcs;

pub use blob::{BlobStore, BlobStoreExt, ContainerMetadata, ObjectMetadata};
pub use broadcast::Broadcast;
pub use config::Config;
#[cfg(feature = "docstore")]
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
#[cfg(feature = "sql")]
pub use table::TableStore;
// Version-control wire names stay scoped to the vcs capability.
pub use vcs::Vcs;
#[cfg(target_arch = "wasm32")]
pub use vcs::WasiVcs;
