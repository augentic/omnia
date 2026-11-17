//! # WASI Blobstore WIT implementation

// Bindings for `omnia:blobstore`.
mod generated {
    #![allow(missing_docs, clippy::same_length_and_capacity, reason = "wit-bindgen output")]
    wit_bindgen::generate!({
        world: "imports",
        path: "wit",
        generate_all,
    });
}

/// `omnia:blobstore`, the interfaces behind the SDK's `BlobStore`.
pub mod omnia {
    pub use crate::guest::generated::omnia::blobstore::*;
}

pub use self::generated::{wit_future, wit_stream};
