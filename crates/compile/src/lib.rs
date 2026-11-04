#![doc = include_str!("../README.md")]
#![cfg(not(target_arch = "wasm32"))]

#[cfg(feature = "cranelift")]
mod compile;
mod options;

#[cfg(feature = "cranelift")]
pub use self::compile::compile;
pub use self::options::CompileOptions;
