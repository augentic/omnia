# omnia-compile

The ahead-of-time compiler of [omnia](https://github.com/augentic/omnia):
[`compile`] turns a `wasm32-wasip2` component into the artifact the runtime
deserializes instead of compiling at startup, and [`CompileOptions`] is the
one value of settings that shape such an artifact.

A pre-compiled component loads only into an engine configured with the
values it was compiled under, so those settings travel as one value and are
applied by one body, [`CompileOptions::configure`]: the compiler takes it
explicitly, and the runtime's engine applies the copy it loaded from the
environment (`RuntimeOptions::compile_options()` in `omnia-core`). That
parity is what lets the loading engine accept the artifact.

The crate is a leaf over `wasmtime` with nothing else of omnia beneath it,
so a build pipeline that pre-compiles a guest links the compiler alone. The
`cranelift` feature (default) is the backend; without it the crate is
`CompileOptions` alone, which is all a runtime that only loads artifacts
needs. omnia's `jit` feature forwards to it.

Embedders reach the compiler as `omnia::compile` (with the default `jit`
feature) and the settings as `omnia::CompileOptions`. Depend on
`omnia-compile` directly only from a tool that compiles and never runs a
guest — a `build.rs`, or a binary built for one.

[`compile`]: https://docs.rs/omnia-compile/latest/omnia_compile/fn.compile.html
[`CompileOptions`]: https://docs.rs/omnia-compile/latest/omnia_compile/struct.CompileOptions.html
[`CompileOptions::configure`]: https://docs.rs/omnia-compile/latest/omnia_compile/struct.CompileOptions.html#method.configure
