//! # WASI Messaging WIT implementation

// Bindings for the `wasi:messaging` world.
// See (<https://github.com/WebAssembly/wasi-messaging/>)
mod generated {
    #![allow(missing_docs)]
    #![allow(clippy::same_length_and_capacity)]

    wit_bindgen::generate!({
        world: "messaging",
        path: "wit",
        additional_derives: [Clone],
        generate_all,
        pub_export_macro: true,
        // async: [
        //     "wasi:messaging/producer@0.2.0-draft#send",
        //     "wasi:messaging/request-reply@0.2.0-draft#request",
        //     "wasi:messaging/incoming-handler@0.2.0-draft#handle",
        // ],
    });
}

pub use self::generated::exports::wasi::messaging::*;
pub use self::generated::wasi::messaging::*;
pub use self::generated::*;
