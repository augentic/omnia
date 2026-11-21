# WIT Packages Reference

Reference for the `omnia:*` WIT packages Omnia publishes to the `omnia.host` wasm-pkg registry: the packages, how a consumer fetches one, the version rules, the conventions a package's WIT keeps, and how a release publishes them. The authoritative WIT for each package is the `wit/` directory of the crate that owns it.

- [Packages](#packages)
- [Consuming a package](#consuming-a-package)
- [Versions](#versions)
- [Authoring a package](#authoring-a-package)
- [Publishing](#publishing)

## Packages

| Package | Version | WIT |
| ------- | ------- | --- |
| `omnia:types` | `0.1.0` | [`crates/wasi-types/wit`](../../crates/wasi-types/wit/types.wit) |
| `omnia:blobstore` | `0.1.0` | [`crates/wasi-blobstore/wit`](../../crates/wasi-blobstore/wit/blobstore.wit) |
| `omnia:config` | `0.1.0` | [`crates/wasi-config/wit`](../../crates/wasi-config/wit/config.wit) |
| `omnia:docstore` | `0.1.0` | [`crates/wasi-docstore/wit`](../../crates/wasi-docstore/wit/docstore.wit) |
| `omnia:identity` | `0.1.0` | [`crates/wasi-identity/wit`](../../crates/wasi-identity/wit/identity.wit) |
| `omnia:keyvalue` | `0.1.0` | [`crates/wasi-keyvalue/wit`](../../crates/wasi-keyvalue/wit/keyvalue.wit) |
| `omnia:messaging` | `0.1.0` | [`crates/wasi-messaging/wit`](../../crates/wasi-messaging/wit/messaging.wit) |
| `omnia:model` | `0.1.0` | [`crates/wasi-model/wit`](../../crates/wasi-model/wit/model.wit) |
| `omnia:otel` | `0.1.0` | [`crates/wasi-otel/wit`](../../crates/wasi-otel/wit/otel.wit) |
| `omnia:plugins` | `0.1.0` | [`crates/plugin/wit`](../../crates/plugin/wit/plugins.wit) |
| `omnia:sql` | `0.1.0` | [`crates/wasi-sql/wit`](../../crates/wasi-sql/wit/sql.wit) |
| `omnia:vault` | `0.1.0` | [`crates/wasi-vault/wit`](../../crates/wasi-vault/wit/vault.wit) |
| `omnia:vcs` | `0.1.0` | [`crates/wasi-vcs/wit`](../../crates/wasi-vcs/wit/vcs.wit) |
| `omnia:websocket` | `0.1.0` | [`crates/wasi-websocket/wit`](../../crates/wasi-websocket/wit/websocket.wit) |

- Every package but `omnia:types` takes its `error` from `omnia:types/types`. `omnia:model` and `omnia:vcs` also use `wasi:filesystem` and `wasi:clocks` 0.3.2, which `wasi.dev` serves.
- A package carries its unstable interfaces under their `@unstable` gates. A consumer that enables no feature binds the stable interfaces alone, and one that enables a gate (`--features`, or `--all-features` as every host does) binds that interface too. [WASI Interfaces](wasi-interfaces.md) lists the gates.
- A release's description is the first paragraph of its package docs. Its license, source, homepage, and authors are its crate's.

## Consuming a package

`omnia.host` serves the registry metadata that points wkg at `ghcr.io/augentic/omnia/<name>`, at `/.well-known/wasm-pkg/registry.json`:

```json
{ "preferredProtocol": "oci", "oci": { "registry": "ghcr.io", "namespacePrefix": "augentic/" } }
```

Route the `omnia` namespace in a wasm-pkg configuration, either `~/.config/wasm-pkg/config.toml` or a file passed with `--config`. A file passed with `--config` gets none of wkg's default routes, so it routes `wasi` too, for the `wasi:*` packages `omnia:model` and `omnia:vcs` use:

```toml
[namespace_registries]
omnia = "omnia.host"
wasi = "wasi.dev"
```

Reading a release needs no credentials. `wkg get` fetches one package's WIT:

```bash
wkg get omnia:keyvalue@0.1.0 --format wit --output keyvalue.wit
```

A component whose own `wit/` imports an `omnia:*` interface runs `wkg fetch`. It fetches every package the component uses into `wit/deps` and pins each in `wkg.lock`.

## Versions

- A package's version is its own and moves independently of the crate version.
- A published release is immutable. `mise run wit-verify` fails when the registry holds a package's version with WIT, docs included, that differs from the tree's. CI's `WIT` job runs it on every push. Give a changed package a new version.
- While a package is on `0.x`, a breaking change bumps its minor version and an additive change its patch version. `wkg publish` refuses a release that is not additive over the releases of its series, such as `0.1.1` over `0.1.0`.
- A new version changes the package's `package` line, and every `use` or `import` that names the package in another package's WIT. `mise run wit-sync` then rewrites the copies, and `mise run bindgen` the guest bindings. Each host crate's `bindgen!` names an interface without its version, so a new version alone changes no Rust.

## Authoring a package

- The package docs, the `///` above the `package` line, open with one summary sentence. It becomes the release's description.
- wit-parser reads a plain `//` comment as the next item's docs, so it reaches the published package and the guest bindings. A maintainer's note lives outside the WIT, and `mise run wit-check` refuses a plain comment in a published package.
- An `@unstable` interface repeats its gate on every type and `use` inside it. wit-parser excludes a gated interface's types only where each carries the interface's gate.
- A crate copies each package its WIT uses into its own `wit/deps`, since `bindgen!` reads the crate's `wit/` when the published crate builds. A copy's owner is the published package of its name, or the WASI package the locked `wasip3` ships. Edit the owner, then run `mise run wit-sync`, which rewrites every copy. `mise run wit-check` fails on a stale one.
- `mise run wit-check` is the offline gate. It is the `xtask` suite, so the workspace test runs it in CI. It checks the copies and comments, and parses every `wit` directory with no features and with all of them. It encodes every release, and fails when the guest bindings differ from a regeneration.

## Publishing

The **Publish Release** workflow publishes the WIT once the release is tagged. Its `CI` job runs `wit-check` with the workspace test. Its `Publish WIT` job then checks out the tag and runs `cargo xtask publish` with the workflow's `GITHUB_TOKEN`. `publish` goes in two steps:

1. Read each release the registry holds, and fail before publishing anything if one differs from the tree.
2. Publish each release the registry lacks, `omnia:types` first.

A run that stops partway is re-run as it is, since a release already published with the same WIT is skipped.

`mise run wit-publish` runs the same steps locally. It routes through the repository's `.wkg-config.toml`. It reads the credentials from `WKG_REGISTRY_GHCR_IO_AUTH_BEARER`, or else from `docker login ghcr.io`, with a token that has `write:packages`. `mise run wit-verify` reads a private package's releases with the same credentials.

A new package on `ghcr.io` starts private, so make it public in its package settings after its first release. A package first published from another repository needs `augentic/omnia` granted write access under its **Manage Actions access** settings before the workflow can publish to it.
