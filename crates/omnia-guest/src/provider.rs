//! Provider declaration macro for WASI-backed guests.

/// Declares a unit provider struct with empty capability implementations.
///
/// Each listed capability trait gets an `impl <Cap> for <Name> {}`, picking up
/// the WASI-backed default method bodies — so the expansion only compiles on
/// `wasm32` targets. Native tests supply their own mock providers instead.
///
/// ```rust,ignore
/// omnia_guest::provider! {
///     /// Bare provider backed by the default WASI capability implementations.
///     pub struct Provider: Config + HttpRequest + Identity + Publish + StateStore;
/// }
/// ```
#[macro_export]
macro_rules! provider {
    (
        $(#[$attr:meta])*
        $vis:vis struct $name:ident: $first:ident $(+ $capability:ident)*;
    ) => {
        $(#[$attr])*
        #[derive(::core::clone::Clone)]
        $vis struct $name;

        impl $crate::$first for $name {}
        $(impl $crate::$capability for $name {})*
    };
}
