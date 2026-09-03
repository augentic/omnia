//! Support for the `delegate!` expansion: one marker trait per capability
//! whose only method hands back `&Self`.
//!
//! `delegate!` calls `self.field.__as()` inside a block that imports exactly
//! one of these traits. Method resolution auto-derefs, so the field may hold
//! the double directly or behind an `Arc`, and the returned reference feeds a
//! fully qualified trait call with no ambiguity between capabilities that
//! share method names (`get`, `send`, `delete`).

pub use omnia_guest;

macro_rules! as_capability {
    ($($name:ident => $capability:ident),* $(,)?) => {
        $(
            pub trait $name {
                fn __as(&self) -> &Self;
            }

            impl<T: omnia_guest::$capability + ?Sized> $name for T {
                fn __as(&self) -> &Self {
                    self
                }
            }
        )*
    };
}

as_capability! {
    AsBlobStore => BlobStore,
    AsBroadcast => Broadcast,
    AsConfig => Config,
    AsDocumentStore => DocumentStore,
    AsHttpRequest => HttpRequest,
    AsIdentity => Identity,
    AsModel => Model,
    AsPlugins => Plugins,
    AsPublish => Publish,
    AsStateStore => StateStore,
    AsTableStore => TableStore,
}
