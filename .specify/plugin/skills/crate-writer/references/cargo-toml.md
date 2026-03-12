# Cargo.toml Template

## Base Template

Every generated crate starts from this template. All dependencies use `workspace = true` -- never specify versions directly.

```toml
[package]
name = "<crate-name>"
description = "<Short description of domain logic>"
readme = "README.md"
publish = false
authors.workspace = true
categories.workspace = true
edition.workspace = true
keywords.workspace = true
repository.workspace = true
rust-version.workspace = true
version.workspace = true

[lints]
workspace = true

[lib]
crate-type = ["lib"]

[dependencies]
anyhow.workspace = true
omnia-sdk.workspace = true
serde = { workspace = true, features = ["derive"] }
serde_json.workspace = true
tracing.workspace = true
# Add conditional dependencies per the decision tree below

[dev-dependencies]
tokio.workspace = true
```

## Dependency Decision Tree

Add dependencies based on what the handler needs:

### Always Required

| Dependency              | Purpose                               |
| ----------------------- | ------------------------------------- |
| `anyhow`                | Error chaining with `.context()`      |
| `omnia-sdk`             | Handler trait, Error, provider traits |
| `serde` (with `derive`) | Serialization/deserialization         |
| `serde_json`            | JSON parsing and serialization        |
| `tracing`               | Logging and metrics                   |

### When Handler Uses HttpRequest

Add these when the handler makes outbound HTTP calls (`P: HttpRequest`):

```toml
bytes.workspace = true
http.workspace = true
http-body.workspace = true
http-body-util.workspace = true
```

### When Handler Has Domain Errors

Add when the crate defines domain error enums with `thiserror`:

```toml
thiserror.workspace = true
```

### When Handler Parses XML

Add when artifacts indicate XML input:

```toml
quick-xml = { workspace = true, features = ["serde", "serialize"] }
```

### When Handler Uses Date/Time

```toml
chrono = { workspace = true, features = ["serde"] }
```

### When Handler Uses Timezone Conversion

```toml
chrono-tz.workspace = true
```

### When Handler Uses Integer-Backed Enums

```toml
serde_repr.workspace = true
```

### When Handler Parses Query Strings

Add when a handler uses `type Input = Option<String>` with URL-encoded parameters:

```toml
serde_urlencoded.workspace = true
```

### When Handler Uses Streaming/Pagination

```toml
async-stream = "0.3.6"
futures.workspace = true
```

### When Handler Uses URL Encoding

```toml
percent-encoding = "2.3.2"
```

## Dev Dependencies

Always include `tokio` for async test runtime:

```toml
[dev-dependencies]
tokio.workspace = true
```

Optionally include `augentic-test` when using the replay test framework:

```toml
[dev-dependencies]
augentic-test.workspace = true
tokio.workspace = true
```

## Complete Example: r9k-adapter

```toml
[package]
name = "r9k-adapter"
description = "R9K position adapter"
readme = "README.md"
publish = false
authors.workspace = true
categories.workspace = true
edition.workspace = true
keywords.workspace = true
repository.workspace = true
rust-version.workspace = true
version.workspace = true

[lints]
workspace = true

[lib]
crate-type = ["lib"]

[dependencies]
anyhow.workspace = true
bytes.workspace = true
chrono.workspace = true
chrono-tz.workspace = true
http.workspace = true
http-body.workspace = true
http-body-util.workspace = true
quick-xml.workspace = true
serde.workspace = true
serde_json.workspace = true
serde_repr.workspace = true
thiserror.workspace = true
tracing.workspace = true
omnia-sdk.workspace = true

[dev-dependencies]
augentic-test.workspace = true
tokio.workspace = true
```

## Complete Example: cars

```toml
[package]
name = "cars"
description = "CARs and TMP integration domain logic"
readme = "README.md"
version.workspace = true
edition.workspace = true
authors.workspace = true
license.workspace = true
repository.workspace = true
keywords.workspace = true
categories.workspace = true

[lints]
workspace = true

[dependencies]
anyhow.workspace = true
async-stream = "0.3.6"
bytes.workspace = true
chrono.workspace = true
futures.workspace = true
http.workspace = true
http-body.workspace = true
http-body-util.workspace = true
percent-encoding = "2.3.2"
omnia-sdk.workspace = true
serde.workspace = true
serde_json.workspace = true
serde_urlencoded.workspace = true
tracing.workspace = true

[dev-dependencies]
tokio.workspace = true
```

## Rules

1. **Never specify versions** for workspace dependencies -- use `workspace = true`
2. **Never add `uuid`** -- ID generation is the caller's responsibility
3. **Never add `tokio` to `[dependencies]`** -- only in `[dev-dependencies]` for tests
4. **Always include `[lints] workspace = true`** -- inherits project-wide lint configuration
5. **Always set `publish = false`** -- domain crates are not published to registries
6. **Use `crate-type = ["lib"]`** -- crates are libraries consumed by the WASM guest
