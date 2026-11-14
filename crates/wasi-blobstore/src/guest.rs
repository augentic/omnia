//! # WASI Blobstore WIT implementation

// Bindings for `omnia:blobstore` and the `wasi:blobstore` draft.
// See (<https://github.com/WebAssembly/wasi-blobstore/>)
mod generated {
    #![allow(missing_docs, clippy::same_length_and_capacity, reason = "wit-bindgen output")]
    wit_bindgen::generate!({
        world: "imports",
        path: "wit",
        generate_all,
    });
}

/// `omnia:blobstore`, the interface the SDK's `BlobStore` calls.
pub mod omnia {
    pub use crate::guest::generated::omnia::blobstore::store;
}

pub use self::generated::wasi::blobstore::*;
