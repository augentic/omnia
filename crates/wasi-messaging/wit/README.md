# WebAssembly Interface Types (WIT)

`messaging.wit` is `omnia:messaging`, owned by this crate and edited here.
Every backend serves `types` and `producer`, the `core` world.
`request-reply`, `consumer`, `log`, and `admin` are optional, and `imports`
adds them. A subscribing guest exports `incoming-handler`: `messaging-core`
adds it to `core`, `messaging-request-reply` adds `request-reply` to that,
and `messaging` adds it to `imports`.
