#![doc = include_str!("../README.md")]

//! # WASI Types
//!
//! This crate owns `omnia:types`, the WIT every interface crate vendors, and
//! binds it once so every host shares one [`Error`].

#[expect(missing_docs, reason = "bindgen documents no host trait or linker function")]
mod generated {
    pub use self::omnia::types::types::Error;

    wasmtime::component::bindgen!({
        world: "imports",
        path: "wit",
        trappable_error_type: {
            "omnia:types/types.error" => Error,
        },
    });
}

use std::fmt;

use omnia_core::HostView;
use wasmtime::component::ResourceTableError;

pub use self::generated::Error;
pub use self::generated::omnia::types::types;

impl<C: ?Sized> types::Host for HostView<'_, C> {
    fn convert_error(&mut self, err: Error) -> wasmtime::Result<Error> {
        Ok(err)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for Error {}

// a backend picks the kind by returning an `Error`; any other failure is
// `other`, with its full context chain
impl From<anyhow::Error> for Error {
    fn from(err: anyhow::Error) -> Self {
        err.downcast::<Self>().unwrap_or_else(|err| Self::Other(format!("{err:#}")))
    }
}

impl From<wasmtime::Error> for Error {
    fn from(err: wasmtime::Error) -> Self {
        Self::Other(format!("{err:#}"))
    }
}

impl From<ResourceTableError> for Error {
    fn from(err: ResourceTableError) -> Self {
        Self::Other(err.to_string())
    }
}
