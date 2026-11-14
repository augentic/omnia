//! # `omnia:plugins` Guest

// Bindings for the `omnia:plugins` world.
mod generated {
    #![expect(clippy::same_length_and_capacity, reason = "wit-bindgen output")]

    wit_bindgen::generate!({
        world: "imports",
        path: "wit",
        generate_all,
    });
}

pub use self::generated::omnia::plugins::*;
