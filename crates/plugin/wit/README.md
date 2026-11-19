# WebAssembly Interface Types (WIT)

`plugins.wit` is `omnia:plugins`, owned by this crate and edited here.
Every backend serves `types` and `loader`, the `core` world. `imports`
includes `core`, and an optional interface joins it there.

`crates/test-programs/wit/deps/plugins/plugins.wit` is a copy, and the
`wit_copies` test holds it byte-equal to this file.
