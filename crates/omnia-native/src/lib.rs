#![doc = include_str!("../README.md")]
#![cfg(not(target_arch = "wasm32"))]

mod direct;
mod serve;

pub use omnia_backend::{keyvalue, vault};

pub use self::direct::Direct;
pub use self::serve::{RouterHandler, Serve};
