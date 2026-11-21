# WebAssembly Interface Types (WIT)

`types.wit` is `omnia:types`, owned by this crate and edited here.

Every `omnia:*` package that uses it resolves it from a copy in its own
`wit/deps/types.wit`. A copy changes only with this file. `mise run wit-sync`
rewrites each one from it, and `mise run wit-check` fails when one drifts.
