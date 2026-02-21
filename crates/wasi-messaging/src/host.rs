pub mod default_impl;
mod producer_impl;
mod request_reply_impl;
mod resource;
mod server;
mod types_impl;

mod generated {
    #![allow(missing_docs)]

    pub use wasi::messaging::types::Error;

    pub use crate::host::resource::{ClientProxy, MessageProxy, RequestOptions};

    wasmtime::component::bindgen!({
        world: "messaging",
        path: "wit",
        imports: {
            // "wasi:messaging/types.[static]client.connect": store | tracing | trappable,
            default:  store | tracing | trappable,
        },
        exports: {
            default: store | tracing | trappable,
        },
        with: {
            "wasi:messaging/request-reply.request-options": RequestOptions,
            "wasi:messaging/types.client": ClientProxy,
            "wasi:messaging/types.message": MessageProxy,
        },
        trappable_error_type: {
            "wasi:messaging/types.error" => Error,
        },
        // include_generated_code_from_file: true,
    });
}

use std::fmt::Debug;
use std::sync::Arc;

pub use omnia::FutureResult;
use omnia::{Host, Server, State};
use wasmtime::component::{HasData, Linker};
use wasmtime_wasi::{ResourceTable, ResourceTableError};

pub use self::default_impl::MessagingDefault;
pub use self::generated::Messaging;
pub use self::generated::wasi::messaging::types::Error;
use self::generated::wasi::messaging::{producer, request_reply, types};
pub use self::resource::*;

/// Result type for messaging operations.
pub type Result<T, E = Error> = anyhow::Result<T, E>;

/// Host-side service for `wasi:messaging`.
#[derive(Debug)]
pub struct WasiMessaging;

impl HasData for WasiMessaging {
    type Data<'a> = WasiMessagingCtxView<'a>;
}

impl<T> Host<T> for WasiMessaging
where
    T: WasiMessagingView + 'static,
{
    fn add_to_linker(linker: &mut Linker<T>) -> anyhow::Result<()> {
        producer::add_to_linker::<_, Self>(linker, T::messaging)?;
        request_reply::add_to_linker::<_, Self>(linker, T::messaging)?;
        Ok(types::add_to_linker::<_, Self>(linker, T::messaging)?)
    }
}

impl<S> Server<S> for WasiMessaging
where
    S: State,
    S::StoreCtx: WasiMessagingView,
{
    async fn run(&self, state: &S) -> anyhow::Result<()> {
        server::run(state).await
    }
}

/// A trait which provides internal WASI Messaging state.
///
/// This is implemented by the `T` in `Linker<T>` — a single type shared across
/// all WASI components for the runtime build.
pub trait WasiMessagingView: Send {
    /// Return a [`WasiMessagingCtxView`] from mutable reference to self.
    fn messaging(&mut self) -> WasiMessagingCtxView<'_>;
}

/// View into [`WasiMessagingCtx`] implementation and [`ResourceTable`].
pub struct WasiMessagingCtxView<'a> {
    /// Mutable reference to the WASI Messaging context.
    pub ctx: &'a mut dyn WasiMessagingCtx,

    /// Mutable reference to table used to manage resources.
    pub table: &'a mut ResourceTable,
}

/// A trait which provides internal WASI Messaging context.
///
/// This is implemented by the resource-specific provider of messaging
/// functionality. For example, a NATS, or a Kafka broker.
#[allow(unused)]
pub trait WasiMessagingCtx: Debug + Send + Sync + 'static {
    /// Connect to the messaging system and return a client proxy.
    ///
    /// # Errors
    ///
    /// Returns an error if the connection fails.
    fn connect(&self) -> FutureResult<Arc<dyn Client>>;

    /// Create a new message with the given payload.
    ///
    /// # Errors
    ///
    /// Returns an error if message creation fails.
    fn new_message(&self, data: Vec<u8>) -> anyhow::Result<Arc<dyn Message>>;

    /// Set the content-type on a message.
    ///
    /// # Errors
    ///
    /// Returns an error if the content-type setting fails.
    fn set_content_type(
        &self, message: Arc<dyn Message>, content_type: String,
    ) -> anyhow::Result<Arc<dyn Message>>;

    /// Set the payload on a message.
    ///
    /// # Errors
    ///
    /// Returns an error if the payload setting fails.
    fn set_payload(
        &self, message: Arc<dyn Message>, data: Vec<u8>,
    ) -> anyhow::Result<Arc<dyn Message>>;

    /// Append a key-value pair to the metadata of a message.
    ///
    /// # Errors
    ///
    /// Returns an error if the metadata addition fails.
    fn add_metadata(
        &self, message: Arc<dyn Message>, key: String, value: String,
    ) -> anyhow::Result<Arc<dyn Message>>;

    /// Set all the metadata on a message.
    ///
    /// # Errors
    ///
    /// Returns an error if the metadata setting fails.
    fn set_metadata(
        &self, message: Arc<dyn Message>, metadata: Metadata,
    ) -> anyhow::Result<Arc<dyn Message>>;

    /// Remove a key-value pair from the metadata of a message.
    ///
    /// # Errors
    ///
    /// Returns an error if the metadata removal fails.
    fn remove_metadata(
        &self, message: Arc<dyn Message>, key: String,
    ) -> anyhow::Result<Arc<dyn Message>>;
}

/// `anyhow::Error` to `Error` mapping
impl From<anyhow::Error> for Error {
    fn from(err: anyhow::Error) -> Self {
        Self::Other(err.to_string())
    }
}

/// `ResourceTableError` to `Error` mapping
impl From<ResourceTableError> for Error {
    fn from(err: ResourceTableError) -> Self {
        Self::Other(err.to_string())
    }
}

/// `wasmtime::Error` to `Error` mapping
impl From<wasmtime::Error> for Error {
    fn from(err: wasmtime::Error) -> Self {
        Self::Other(err.to_string())
    }
}

/// Implementation of the `WasiMessagingView` trait for the store context.
#[macro_export]
macro_rules! omnia_wasi_view {
    ($store_ctx:ty, $field_name:ident) => {
        impl omnia_wasi_messaging::WasiMessagingView for $store_ctx {
            fn messaging(&mut self) -> omnia_wasi_messaging::WasiMessagingCtxView<'_> {
                omnia_wasi_messaging::WasiMessagingCtxView {
                    ctx: &mut self.$field_name,
                    table: &mut self.table,
                }
            }
        }
    };
}
