# WebAssembly Interface Types (WIT)

`otel.wit` is `omnia:otel`, owned by this crate and edited here.
Every backend serves `types`, `traces`, and `metrics`, the `core` world.
`logs` is optional, and `imports` adds it. It is `@unstable`: a guest
binds it by enabling the `logs` feature, and every host links it.
