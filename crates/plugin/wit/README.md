# WebAssembly Interface Types (WIT)

`plugins.wit` is `omnia:plugins`, owned by this crate and edited here.
Every backend serves `types` and `loader`, the `core` world. `imports`
includes `core`, and an optional interface joins it there.

`crates/test-programs/wit/deps/plugins/plugins.wit` is a copy. `mise run
wit-sync` rewrites it from this file, and `mise run wit-check` fails when it
drifts.
