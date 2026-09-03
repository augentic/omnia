//! `provider!` and `delegate!`.
//!
//! `omnia_test::provider!` is the native twin of `omnia_guest::provider!`:
//! the same token and grammar, so a test's declaration differs from
//! `src/lib.rs` by the crate path alone. The production macro expands to
//! empty impls over the WASI defaults for `wasm32`; this one expands to a
//! double per capability for the native handler rung.

/// Declares a provider struct holding one default double per capability.
///
/// Accepts `omnia_guest::provider!`'s grammar verbatim. Each capability
/// becomes a `pub` field named for it, seeded through a consuming builder of
/// the same name, and the capability impl delegates to that field. `StateStore` and
/// `BlobStore` share one `storage` field, as one `Memory` serves both — the
/// shape a production provider's single storage backend has. The struct
/// derives `Clone`, `Debug`, and `Default`.
///
/// | Capability | Field | Double |
/// | ---------- | ----- | ------ |
/// | `Config` | `config` | [`MapConfig`](crate::guest::MapConfig) |
/// | `HttpRequest` | `http` | [`MatchedHttp`](crate::guest::MatchedHttp) |
/// | `Identity` | `identity` | [`FixedIdentity`](crate::guest::FixedIdentity) |
/// | `Publish` | `publish` | [`Sink`](crate::guest::Sink) |
/// | `Broadcast` | `broadcast` | [`Sink`](crate::guest::Sink) |
/// | `StateStore` | `storage` | [`Memory`](crate::guest::Memory) |
/// | `BlobStore` | `storage` | [`Memory`](crate::guest::Memory) |
/// | `DocumentStore` | `docs` | [`MemoryDocs`](crate::guest::MemoryDocs) |
/// | `TableStore` | `tables` | [`ScriptedTables`](crate::guest::ScriptedTables) |
/// | `Model` | `model` | [`Scripted`](crate::guest::Scripted) |
/// | `Plugins` | `plugins` | [`ScriptedLoader`](crate::guest::ScriptedLoader) |
///
/// ```rust
/// use omnia_guest::model::{Model as _, Request};
/// use omnia_guest::{Config, StateStore as _};
/// use omnia_test::guest::{MapConfig, Scripted};
///
/// omnia_test::provider! {
///     /// The provider a handler test drives.
///     pub struct Provider: Config + StateStore + Model;
/// }
///
/// # tokio::runtime::Runtime::new().unwrap().block_on(async {
/// let provider = Provider::default()
///     .config(MapConfig::default().with([("region", "eu")]))
///     .model(Scripted::answering(["ok"]));
/// provider.storage.insert_state("seen", b"1");
///
/// assert_eq!(Config::get(&provider, "region").await.unwrap(), "eu");
/// let request = Request::builder().messages(vec![]).build();
/// assert_eq!(provider.complete(request).await.unwrap().answer, "ok");
/// assert_eq!(provider.storage.state("seen"), Some(b"1".to_vec()));
/// # });
/// ```
#[macro_export]
macro_rules! provider {
    (
        $(#[$attr:meta])*
        $vis:vis struct $name:ident: $first:ident $(+ $capability:ident)*;
    ) => {
        $crate::__provider!(@munch [$(#[$attr])* $vis $name] [] $first $($capability)*);
    };
}

// `@munch` resolves each capability to `(Capability field Double)`; `@fields`
// then walks that list once, emitting a field per distinct name — the shared
// `storage` field is declared by whichever of `StateStore` / `BlobStore` comes
// first (the trailing `no` / `yes` token tracks it) and delegated to by both.
#[doc(hidden)]
#[macro_export]
macro_rules! __provider {
    (@munch $head:tt [$($acc:tt)*] Config $($rest:ident)*) => {
        $crate::__provider!(@munch $head [$($acc)* (Config config $crate::guest::MapConfig)] $($rest)*);
    };
    (@munch $head:tt [$($acc:tt)*] HttpRequest $($rest:ident)*) => {
        $crate::__provider!(@munch $head [$($acc)* (HttpRequest http $crate::guest::MatchedHttp)] $($rest)*);
    };
    (@munch $head:tt [$($acc:tt)*] Identity $($rest:ident)*) => {
        $crate::__provider!(@munch $head [$($acc)* (Identity identity $crate::guest::FixedIdentity)] $($rest)*);
    };
    (@munch $head:tt [$($acc:tt)*] Publish $($rest:ident)*) => {
        $crate::__provider!(@munch $head [$($acc)* (Publish publish $crate::guest::Sink)] $($rest)*);
    };
    (@munch $head:tt [$($acc:tt)*] Broadcast $($rest:ident)*) => {
        $crate::__provider!(@munch $head [$($acc)* (Broadcast broadcast $crate::guest::Sink)] $($rest)*);
    };
    (@munch $head:tt [$($acc:tt)*] StateStore $($rest:ident)*) => {
        $crate::__provider!(@munch $head [$($acc)* (StateStore storage $crate::guest::Memory)] $($rest)*);
    };
    (@munch $head:tt [$($acc:tt)*] BlobStore $($rest:ident)*) => {
        $crate::__provider!(@munch $head [$($acc)* (BlobStore storage $crate::guest::Memory)] $($rest)*);
    };
    (@munch $head:tt [$($acc:tt)*] DocumentStore $($rest:ident)*) => {
        $crate::__provider!(@munch $head [$($acc)* (DocumentStore docs $crate::guest::MemoryDocs)] $($rest)*);
    };
    (@munch $head:tt [$($acc:tt)*] TableStore $($rest:ident)*) => {
        $crate::__provider!(@munch $head [$($acc)* (TableStore tables $crate::guest::ScriptedTables)] $($rest)*);
    };
    (@munch $head:tt [$($acc:tt)*] Model $($rest:ident)*) => {
        $crate::__provider!(@munch $head [$($acc)* (Model model $crate::guest::Scripted)] $($rest)*);
    };
    (@munch $head:tt [$($acc:tt)*] Plugins $($rest:ident)*) => {
        $crate::__provider!(@munch $head [$($acc)* (Plugins plugins $crate::guest::ScriptedLoader)] $($rest)*);
    };
    (@munch $head:tt [$($acc:tt)*] $unknown:ident $($rest:ident)*) => {
        ::core::compile_error!(::core::concat!(
            "`", ::core::stringify!($unknown), "` is not a capability `provider!` has a double for"
        ));
    };
    (@munch $head:tt [$($acc:tt)*]) => {
        $crate::__provider!(@fields $head [$($acc)*] [] [] no);
    };

    (@fields $head:tt [($capability:ident storage $double:ty) $($rest:tt)*] [$($fields:tt)*] [$($fwd:tt)*] no) => {
        $crate::__provider!(@fields $head [$($rest)*]
            [$($fields)* (storage $double, "The `StateStore` / `BlobStore` double: one shared `Memory`.")]
            [$($fwd)* ($capability storage)] yes);
    };
    (@fields $head:tt [($capability:ident storage $double:ty) $($rest:tt)*] [$($fields:tt)*] [$($fwd:tt)*] yes) => {
        $crate::__provider!(@fields $head [$($rest)*] [$($fields)*] [$($fwd)* ($capability storage)] yes);
    };
    (@fields $head:tt [($capability:ident $field:ident $double:ty) $($rest:tt)*] [$($fields:tt)*] [$($fwd:tt)*] $storage:ident) => {
        $crate::__provider!(@fields $head [$($rest)*]
            [$($fields)* ($field $double, ::core::concat!("The `", ::core::stringify!($capability), "` double."))]
            [$($fwd)* ($capability $field)] $storage);
    };
    (
        @fields [$(#[$attr:meta])* $vis:vis $name:ident] []
        [$(($field:ident $double:ty, $doc:expr))*] [$(($capability:ident $fwd_field:ident))*] $storage:ident
    ) => {
        $(#[$attr])*
        #[derive(::core::clone::Clone, ::core::fmt::Debug, ::core::default::Default)]
        $vis struct $name {
            $(
                #[doc = $doc]
                pub $field: $double,
            )*
        }

        impl $name {
            $(
                #[doc = ::core::concat!("Replaces the `", ::core::stringify!($field), "` double.")]
                #[must_use]
                pub fn $field(mut self, $field: $double) -> Self {
                    self.$field = $field;
                    self
                }
            )*
        }

        $crate::delegate!(impl $name { $($capability => $fwd_field,)* });
    };
}

/// Implements capability traits for a provider by delegating every method
/// to a field.
///
/// Each body entry names one or more capabilities and the field that
/// serves them; the field may hold the implementation directly or behind
/// an `Arc`. A generic header goes in square brackets so the macro can
/// find the type that follows it:
///
/// ```rust
/// use std::sync::Arc;
/// use omnia_guest::{BlobStore, StateStore};
/// use omnia_test::guest::{Memory, Scripted};
///
/// #[derive(Clone)]
/// struct Provider<S> {
///     model: Scripted,
///     storage: Arc<S>,
/// }
///
/// omnia_test::delegate!(impl[S: StateStore + BlobStore + Send + Sync + 'static] Provider<S> {
///     Model => model,
///     StateStore + BlobStore => storage,
/// });
///
/// let provider = Provider { model: Scripted::default(), storage: Arc::new(Memory::default()) };
/// # let _ = provider.clone();
/// ```
#[macro_export]
macro_rules! delegate {
    (impl [$($generics:tt)*] $provider:ty { $($body:tt)* }) => {
        $crate::__delegate_body!([$($generics)*] $provider; $($body)*);
    };
    (impl $provider:ty { $($body:tt)* }) => {
        $crate::__delegate_body!([] $provider; $($body)*);
    };
}

#[doc(hidden)]
#[macro_export]
macro_rules! __delegate_body {
    ($generics:tt $provider:ty;) => {};
    (
        $generics:tt $provider:ty;
        $first:ident $(+ $capability:ident)* => $field:ident $(, $($rest:tt)*)?
    ) => {
        $crate::__delegate_trait!($generics $provider; $first => $field);
        $($crate::__delegate_trait!($generics $provider; $capability => $field);)*
        $crate::__delegate_body!($generics $provider; $($($rest)*)?);
    };
}

#[doc(hidden)]
#[macro_export]
macro_rules! __delegate_trait {
    ([$($generics:tt)*] $provider:ty; Config => $field:ident) => {
        impl<$($generics)*> $crate::guest::__delegate::omnia_guest::Config for $provider {
            fn get(
                &self, key: &str,
            ) -> impl ::core::future::Future<
                Output = $crate::guest::__delegate::omnia_guest::anyhow::Result<::std::string::String>,
            > + Send {
                use $crate::guest::__delegate::AsConfig as _;
                $crate::guest::__delegate::omnia_guest::Config::get(self.$field.__as(), key)
            }
        }
    };

    ([$($generics:tt)*] $provider:ty; Identity => $field:ident) => {
        impl<$($generics)*> $crate::guest::__delegate::omnia_guest::Identity for $provider {
            fn access_token(
                &self, identity: ::std::string::String,
            ) -> impl ::core::future::Future<
                Output = $crate::guest::__delegate::omnia_guest::anyhow::Result<::std::string::String>,
            > + Send {
                use $crate::guest::__delegate::AsIdentity as _;
                $crate::guest::__delegate::omnia_guest::Identity::access_token(
                    self.$field.__as(),
                    identity,
                )
            }
        }
    };

    ([$($generics:tt)*] $provider:ty; Publish => $field:ident) => {
        impl<$($generics)*> $crate::guest::__delegate::omnia_guest::Publish for $provider {
            fn send(
                &self, topic: &str, message: &$crate::guest::__delegate::omnia_guest::Message,
            ) -> impl ::core::future::Future<
                Output = $crate::guest::__delegate::omnia_guest::anyhow::Result<()>,
            > + Send {
                use $crate::guest::__delegate::AsPublish as _;
                $crate::guest::__delegate::omnia_guest::Publish::send(self.$field.__as(), topic, message)
            }
        }
    };

    ([$($generics:tt)*] $provider:ty; Broadcast => $field:ident) => {
        impl<$($generics)*> $crate::guest::__delegate::omnia_guest::Broadcast for $provider {
            fn send(
                &self, name: &str, data: &[u8], sockets: ::core::option::Option<::std::vec::Vec<::std::string::String>>,
            ) -> impl ::core::future::Future<
                Output = $crate::guest::__delegate::omnia_guest::anyhow::Result<()>,
            > + Send {
                use $crate::guest::__delegate::AsBroadcast as _;
                $crate::guest::__delegate::omnia_guest::Broadcast::send(
                    self.$field.__as(),
                    name,
                    data,
                    sockets,
                )
            }
        }
    };

    ([$($generics:tt)*] $provider:ty; HttpRequest => $field:ident) => {
        impl<$($generics)*> $crate::guest::__delegate::omnia_guest::HttpRequest for $provider {
            fn fetch<T>(
                &self, request: $crate::guest::__delegate::omnia_guest::http::Request<T>,
            ) -> impl ::core::future::Future<
                Output = $crate::guest::__delegate::omnia_guest::anyhow::Result<
                    $crate::guest::__delegate::omnia_guest::http::Response<
                        $crate::guest::__delegate::omnia_guest::bytes::Bytes,
                    >,
                >,
            > + Send
            where
                T: $crate::guest::__delegate::omnia_guest::http_body::Body + ::core::any::Any + Send,
                T::Data: Into<::std::vec::Vec<u8>>,
                T::Error: Into<::std::boxed::Box<dyn ::std::error::Error + Send + Sync + 'static>>,
            {
                use $crate::guest::__delegate::AsHttpRequest as _;
                $crate::guest::__delegate::omnia_guest::HttpRequest::fetch(self.$field.__as(), request)
            }
        }
    };

    ([$($generics:tt)*] $provider:ty; StateStore => $field:ident) => {
        impl<$($generics)*> $crate::guest::__delegate::omnia_guest::StateStore for $provider {
            fn get(
                &self, key: &str,
            ) -> impl ::core::future::Future<
                Output = $crate::guest::__delegate::omnia_guest::anyhow::Result<
                    ::core::option::Option<::std::vec::Vec<u8>>,
                >,
            > + Send {
                use $crate::guest::__delegate::AsStateStore as _;
                $crate::guest::__delegate::omnia_guest::StateStore::get(self.$field.__as(), key)
            }

            fn set(
                &self, key: &str, value: &[u8], ttl_secs: ::core::option::Option<u64>,
            ) -> impl ::core::future::Future<
                Output = $crate::guest::__delegate::omnia_guest::anyhow::Result<
                    ::core::option::Option<::std::vec::Vec<u8>>,
                >,
            > + Send {
                use $crate::guest::__delegate::AsStateStore as _;
                $crate::guest::__delegate::omnia_guest::StateStore::set(
                    self.$field.__as(),
                    key,
                    value,
                    ttl_secs,
                )
            }

            fn delete(
                &self, key: &str,
            ) -> impl ::core::future::Future<
                Output = $crate::guest::__delegate::omnia_guest::anyhow::Result<()>,
            > + Send {
                use $crate::guest::__delegate::AsStateStore as _;
                $crate::guest::__delegate::omnia_guest::StateStore::delete(self.$field.__as(), key)
            }

            fn cas(
                &self, key: &str, expected: ::core::option::Option<&[u8]>, value: &[u8],
            ) -> impl ::core::future::Future<
                Output = ::core::result::Result<(), $crate::guest::__delegate::omnia_guest::CasError>,
            > + Send {
                use $crate::guest::__delegate::AsStateStore as _;
                $crate::guest::__delegate::omnia_guest::StateStore::cas(
                    self.$field.__as(),
                    key,
                    expected,
                    value,
                )
            }

            fn increment(
                &self, key: &str, delta: i64,
            ) -> impl ::core::future::Future<
                Output = $crate::guest::__delegate::omnia_guest::anyhow::Result<i64>,
            > + Send {
                use $crate::guest::__delegate::AsStateStore as _;
                $crate::guest::__delegate::omnia_guest::StateStore::increment(
                    self.$field.__as(),
                    key,
                    delta,
                )
            }
        }
    };

    ([$($generics:tt)*] $provider:ty; BlobStore => $field:ident) => {
        impl<$($generics)*> $crate::guest::__delegate::omnia_guest::BlobStore for $provider {
            fn get(
                &self, container: &str, name: &str,
            ) -> impl ::core::future::Future<
                Output = $crate::guest::__delegate::omnia_guest::anyhow::Result<
                    ::core::option::Option<::std::vec::Vec<u8>>,
                >,
            > + Send {
                use $crate::guest::__delegate::AsBlobStore as _;
                $crate::guest::__delegate::omnia_guest::BlobStore::get(self.$field.__as(), container, name)
            }

            fn put(
                &self, container: &str, name: &str, data: &[u8],
            ) -> impl ::core::future::Future<
                Output = $crate::guest::__delegate::omnia_guest::anyhow::Result<()>,
            > + Send {
                use $crate::guest::__delegate::AsBlobStore as _;
                $crate::guest::__delegate::omnia_guest::BlobStore::put(
                    self.$field.__as(),
                    container,
                    name,
                    data,
                )
            }

            fn delete(
                &self, container: &str, name: &str,
            ) -> impl ::core::future::Future<
                Output = $crate::guest::__delegate::omnia_guest::anyhow::Result<()>,
            > + Send {
                use $crate::guest::__delegate::AsBlobStore as _;
                $crate::guest::__delegate::omnia_guest::BlobStore::delete(
                    self.$field.__as(),
                    container,
                    name,
                )
            }

            fn list(
                &self, container: &str,
            ) -> impl ::core::future::Future<
                Output = $crate::guest::__delegate::omnia_guest::anyhow::Result<
                    ::std::vec::Vec<::std::string::String>,
                >,
            > + Send {
                use $crate::guest::__delegate::AsBlobStore as _;
                $crate::guest::__delegate::omnia_guest::BlobStore::list(self.$field.__as(), container)
            }

            fn get_range(
                &self, container: &str, name: &str, start: u64, end: u64,
            ) -> impl ::core::future::Future<
                Output = $crate::guest::__delegate::omnia_guest::anyhow::Result<::std::vec::Vec<u8>>,
            > + Send {
                use $crate::guest::__delegate::AsBlobStore as _;
                $crate::guest::__delegate::omnia_guest::BlobStore::get_range(
                    self.$field.__as(),
                    container,
                    name,
                    start,
                    end,
                )
            }

            fn object_info(
                &self, container: &str, name: &str,
            ) -> impl ::core::future::Future<
                Output = $crate::guest::__delegate::omnia_guest::anyhow::Result<
                    $crate::guest::__delegate::omnia_guest::ObjectMetadata,
                >,
            > + Send {
                use $crate::guest::__delegate::AsBlobStore as _;
                $crate::guest::__delegate::omnia_guest::BlobStore::object_info(
                    self.$field.__as(),
                    container,
                    name,
                )
            }

            fn create_container(
                &self, name: &str,
            ) -> impl ::core::future::Future<
                Output = $crate::guest::__delegate::omnia_guest::anyhow::Result<()>,
            > + Send {
                use $crate::guest::__delegate::AsBlobStore as _;
                $crate::guest::__delegate::omnia_guest::BlobStore::create_container(self.$field.__as(), name)
            }

            fn delete_container(
                &self, name: &str,
            ) -> impl ::core::future::Future<
                Output = $crate::guest::__delegate::omnia_guest::anyhow::Result<()>,
            > + Send {
                use $crate::guest::__delegate::AsBlobStore as _;
                $crate::guest::__delegate::omnia_guest::BlobStore::delete_container(self.$field.__as(), name)
            }

            fn container_exists(
                &self, name: &str,
            ) -> impl ::core::future::Future<
                Output = $crate::guest::__delegate::omnia_guest::anyhow::Result<bool>,
            > + Send {
                use $crate::guest::__delegate::AsBlobStore as _;
                $crate::guest::__delegate::omnia_guest::BlobStore::container_exists(self.$field.__as(), name)
            }

            fn container_info(
                &self, container: &str,
            ) -> impl ::core::future::Future<
                Output = $crate::guest::__delegate::omnia_guest::anyhow::Result<
                    $crate::guest::__delegate::omnia_guest::ContainerMetadata,
                >,
            > + Send {
                use $crate::guest::__delegate::AsBlobStore as _;
                $crate::guest::__delegate::omnia_guest::BlobStore::container_info(self.$field.__as(), container)
            }
        }
    };

    ([$($generics:tt)*] $provider:ty; DocumentStore => $field:ident) => {
        impl<$($generics)*> $crate::guest::__delegate::omnia_guest::DocumentStore for $provider {
            fn get(
                &self, store: &str, id: &str,
            ) -> impl ::core::future::Future<
                Output = $crate::guest::__delegate::omnia_guest::anyhow::Result<
                    ::core::option::Option<$crate::guest::__delegate::omnia_guest::document_store::Document>,
                >,
            > + Send {
                use $crate::guest::__delegate::AsDocumentStore as _;
                $crate::guest::__delegate::omnia_guest::DocumentStore::get(self.$field.__as(), store, id)
            }

            fn insert(
                &self, store: &str, doc: &$crate::guest::__delegate::omnia_guest::document_store::Document,
            ) -> impl ::core::future::Future<
                Output = $crate::guest::__delegate::omnia_guest::anyhow::Result<()>,
            > + Send {
                use $crate::guest::__delegate::AsDocumentStore as _;
                $crate::guest::__delegate::omnia_guest::DocumentStore::insert(self.$field.__as(), store, doc)
            }

            fn put(
                &self, store: &str, doc: &$crate::guest::__delegate::omnia_guest::document_store::Document,
            ) -> impl ::core::future::Future<
                Output = $crate::guest::__delegate::omnia_guest::anyhow::Result<()>,
            > + Send {
                use $crate::guest::__delegate::AsDocumentStore as _;
                $crate::guest::__delegate::omnia_guest::DocumentStore::put(self.$field.__as(), store, doc)
            }

            fn delete(
                &self, store: &str, id: &str,
            ) -> impl ::core::future::Future<
                Output = $crate::guest::__delegate::omnia_guest::anyhow::Result<bool>,
            > + Send {
                use $crate::guest::__delegate::AsDocumentStore as _;
                $crate::guest::__delegate::omnia_guest::DocumentStore::delete(self.$field.__as(), store, id)
            }

            fn query(
                &self, store: &str,
                options: $crate::guest::__delegate::omnia_guest::document_store::QueryOptions,
            ) -> impl ::core::future::Future<
                Output = $crate::guest::__delegate::omnia_guest::anyhow::Result<
                    $crate::guest::__delegate::omnia_guest::document_store::QueryResult,
                >,
            > + Send {
                use $crate::guest::__delegate::AsDocumentStore as _;
                $crate::guest::__delegate::omnia_guest::DocumentStore::query(
                    self.$field.__as(),
                    store,
                    options,
                )
            }
        }
    };

    ([$($generics:tt)*] $provider:ty; TableStore => $field:ident) => {
        impl<$($generics)*> $crate::guest::__delegate::omnia_guest::TableStore for $provider {
            fn query(
                &self, conn_name: ::std::string::String, query: ::std::string::String,
                params: ::std::vec::Vec<$crate::guest::__delegate::omnia_guest::orm::DataType>,
            ) -> impl ::core::future::Future<
                Output = $crate::guest::__delegate::omnia_guest::anyhow::Result<
                    ::std::vec::Vec<$crate::guest::__delegate::omnia_guest::orm::Row>,
                >,
            > + Send {
                use $crate::guest::__delegate::AsTableStore as _;
                $crate::guest::__delegate::omnia_guest::TableStore::query(
                    self.$field.__as(),
                    conn_name,
                    query,
                    params,
                )
            }

            fn exec(
                &self, conn_name: ::std::string::String, query: ::std::string::String,
                params: ::std::vec::Vec<$crate::guest::__delegate::omnia_guest::orm::DataType>,
            ) -> impl ::core::future::Future<
                Output = $crate::guest::__delegate::omnia_guest::anyhow::Result<u32>,
            > + Send {
                use $crate::guest::__delegate::AsTableStore as _;
                $crate::guest::__delegate::omnia_guest::TableStore::exec(
                    self.$field.__as(),
                    conn_name,
                    query,
                    params,
                )
            }
        }
    };

    ([$($generics:tt)*] $provider:ty; Model => $field:ident) => {
        impl<$($generics)*> $crate::guest::__delegate::omnia_guest::Model for $provider {
            fn complete(
                &self, request: $crate::guest::__delegate::omnia_guest::model::Request,
            ) -> impl ::core::future::Future<
                Output = ::core::result::Result<
                    $crate::guest::__delegate::omnia_guest::model::Reply,
                    $crate::guest::__delegate::omnia_guest::model::Error,
                >,
            > + Send {
                use $crate::guest::__delegate::AsModel as _;
                $crate::guest::__delegate::omnia_guest::Model::complete(self.$field.__as(), request)
            }

            fn complete_with<H, F>(
                &self, request: $crate::guest::__delegate::omnia_guest::model::Request, handler: H,
            ) -> impl ::core::future::Future<
                Output = ::core::result::Result<
                    $crate::guest::__delegate::omnia_guest::model::Reply,
                    $crate::guest::__delegate::omnia_guest::model::Error,
                >,
            > + Send
            where
                H: FnMut($crate::guest::__delegate::omnia_guest::model::ToolCall) -> F + Send,
                F: ::core::future::Future<
                    Output = ::core::result::Result<::std::string::String, ::std::string::String>,
                > + Send,
            {
                use $crate::guest::__delegate::AsModel as _;
                $crate::guest::__delegate::omnia_guest::Model::complete_with(
                    self.$field.__as(),
                    request,
                    handler,
                )
            }
        }
    };

    ([$($generics:tt)*] $provider:ty; Plugins => $field:ident) => {
        impl<$($generics)*> $crate::guest::__delegate::omnia_guest::Plugins for $provider {
            fn load(
                &self, plugin: &$crate::guest::__delegate::omnia_guest::plugins::PluginRef,
            ) -> impl ::core::future::Future<
                Output = ::core::result::Result<
                    $crate::guest::__delegate::omnia_guest::plugins::Plugin,
                    $crate::guest::__delegate::omnia_guest::plugins::Error,
                >,
            > + Send {
                use $crate::guest::__delegate::AsPlugins as _;
                $crate::guest::__delegate::omnia_guest::Plugins::load(self.$field.__as(), plugin)
            }
        }
    };
}
