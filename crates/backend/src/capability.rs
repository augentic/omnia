//! Capabilities a backend serves, named without an engine.

use crate::Connect;

/// A capability a backend can serve, implemented by a host's marker type.
///
/// A `wasi-*` crate declares one beside its client trait with
/// [`capability!`](crate::capability).
pub trait Capability: 'static {
    /// The WIT package the capability serves, such as `wasi:keyvalue`.
    const NAME: &'static str;

    /// What a deployment holds for the capability, cloned into every store.
    type Slot: Clone + Send + Sync + 'static;
}

/// A [`Capability`] that a backend of type `T` can serve.
pub trait Serve<T>: Capability {
    /// Builds the capability's slot from `backend`.
    fn slot(backend: &T) -> Self::Slot;
}

/// A tuple of capabilities that a backend of type `T` serves, every one.
///
/// Implemented for tuples of one to four [`Serve<T>`] capabilities.
pub trait Serves<T> {}

macro_rules! serves {
    ($($host:ident),+) => {
        impl<T, $($host: Serve<T>),+> Serves<T> for ($($host,)+) {}
    };
}

serves!(H1);
serves!(H1, H2);
serves!(H1, H2, H3);
serves!(H1, H2, H3, H4);

/// A backend that names the capabilities it serves.
///
/// # Examples
///
/// ```
/// use std::fmt::Debug;
///
/// use omnia_backend::{Backend, Connect, NoOptions};
///
/// pub trait FooClient: Debug + Send + Sync + 'static {}
///
/// /// Host-side service for `omnia:foo`.
/// pub struct WasiFoo;
/// omnia_backend::capability!(WasiFoo, "omnia:foo", FooClient);
///
/// #[derive(Clone, Debug)]
/// struct Memory;
///
/// impl FooClient for Memory {}
///
/// impl Connect for Memory {
///     type ConnectOptions = NoOptions;
///
///     async fn connect_with(_: NoOptions) -> anyhow::Result<Self> {
///         Ok(Self)
///     }
/// }
///
/// impl Backend for Memory {
///     type Hosts = (WasiFoo,);
/// }
/// ```
pub trait Backend: Connect + 'static {
    /// The capabilities the backend serves, as a tuple of host marker types.
    type Hosts: Serves<Self>;
}

/// Declares a host marker type as the [`Capability`] served through a client
/// trait.
///
/// `capability!(Host, "wit:package", ClientTrait)` gives `Host` the slot
/// `Arc<dyn ClientTrait>`, and lets every `ClientTrait + Clone + 'static`
/// backend [`Serve`] it.
#[macro_export]
macro_rules! capability {
    ($host:ty, $name:literal, $client:path) => {
        impl $crate::Capability for $host {
            type Slot = ::std::sync::Arc<dyn $client>;

            const NAME: &'static str = $name;
        }

        impl<T: $client + ::std::clone::Clone + 'static> $crate::Serve<T> for $host {
            fn slot(backend: &T) -> ::std::sync::Arc<dyn $client> {
                ::std::sync::Arc::new(backend.clone())
            }
        }
    };
}
