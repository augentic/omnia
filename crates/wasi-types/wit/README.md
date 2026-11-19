# WebAssembly Interface Types (WIT)

`types.wit` is `omnia:types`, owned by this crate and edited here.

Every `omnia:*` package that uses it resolves it from a copy in its own
`wit/deps/types.wit`. A copy changes only with this file, and the
`wit_copies` test fails when one drifts.
