# WebAssembly Interface Types (WIT)

`blobstore.wit` is `omnia:blobstore`, owned by this crate and edited here.
Every backend serves `types`, `store`, and `container`, the `core` world.
`admin`, `uploads`, and `presign` are optional, and `imports` adds them.
`uploads` and `presign` are `@unstable`: a guest binds one by enabling
the feature of its name, and every host links both.
