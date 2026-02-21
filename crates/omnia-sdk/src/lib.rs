#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]

pub mod api;
mod capabilities;
mod error;

#[cfg(target_arch = "wasm32")]
pub use omnia_guest_macro::*;
#[doc(hidden)]
pub use {anyhow, axum, bytes, http, http_body, tracing};
#[cfg(target_arch = "wasm32")]
#[doc(hidden)]
pub use {
    omnia_wasi_http, omnia_wasi_identity, omnia_wasi_keyvalue, omnia_wasi_messaging,
    omnia_wasi_otel, wasip3, wit_bindgen,
};

pub use crate::api::*;
pub use crate::capabilities::*;
pub use crate::error::*;

/// Checks required environment variables are set, panicking if any are
/// missing.
///
/// # Example
/// ```rust,ignore
/// omnia_sdk::ensure_env!("API_KEY", "SOME_URL");
/// ```
#[macro_export]
macro_rules! ensure_env {
    ($($var:literal),+ $(,)?) => {
        {
            let mut missing = Vec::new();
            $(
                if std::env::var($var).is_err() {
                    missing.push($var);
                }
            )+

            if !missing.is_empty() {
                panic!("Missing required environment variables: {}", missing.join(", "));
            }
        }
    };
}
