# WebAssembly Interface Types (WIT)

`websocket.wit` is `omnia:websocket`, owned by this crate and edited here.
Every backend serves `types` and `connections`, the `core` world.
`broadcast` is optional, and `imports` adds it. A guest that receives
messages exports `incoming-handler`: `websocket-core` adds it to `core`, and
`websocket` adds it to `imports`.
