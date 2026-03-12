# Configuration

Dependencies, workspace setup, standard config templates, and CI/CD workflows for WASM guests.

---

## Version Resolution

This document uses `<latest>` as a placeholder for dependency versions. **Do not use `<latest>` literally in generated files.** At generation time, resolve each placeholder to the actual latest version:

- **`rust-version`**: Use the latest stable Rust version. Run `rustc --version` to determine the current stable release and use that version number.
- **`omnia-*` crates**: All `omnia-*` crates are published on crates.io. Run `cargo search omnia-sdk` to find the latest version. All `omnia-*` crates share the same version — look up one and use that version for all of them.
- **crates.io dependencies**: Run `cargo search <crate-name>` to find the latest version for each dependency.
- **`wasmtime` / `wasmtime-wasi`**: These must match the version used by the `omnia` crate. After resolving the `omnia` version, check its `Cargo.toml` for the wasmtime version it depends on, and use that same version.

---

## Cargo Setup

### .cargo/config.toml

```toml
[net]
git-fetch-with-cli = true
```

All `omnia-*` crates are on crates.io — no private registry configuration is needed.

### Workspace Configuration

Use resolver version 3 in the workspace `Cargo.toml`:

```toml
[workspace]
resolver = "3"
members = ["crates/*"]
```

Define shared package metadata inherited by the root package and domain crates:

```toml
[workspace.package]
authors = ["<org>"]
categories = ["realtime"]
edition = "2024"
exclude = [".*"]
homepage = "<homepage>"
keywords = ["realtime", "<domain>"]
license = "MIT OR Apache-2.0"
readme = "README.md"
repository = "<repo-url>"
rust-version = "<latest>"
version = "0.1.0"
```

Then inherit in the root `[package]` and each crate's `Cargo.toml`:

```toml
[package]
authors.workspace = true
edition.workspace = true
rust-version.workspace = true
version.workspace = true
# ... etc
```

### Workspace Dependencies

```toml
[workspace.dependencies]
# Runtime crates (from crates.io)
# All omnia-* crates share the same version — resolve via `cargo search omnia-sdk`
omnia = "<latest>"
omnia-sdk = "<latest>"
omnia-wasi-config = "<latest>"
omnia-wasi-http = "<latest>"
omnia-wasi-identity = "<latest>"
omnia-wasi-keyvalue = "<latest>"
omnia-wasi-messaging = "<latest>"
omnia-wasi-otel = "<latest>"
omnia-wasi-sql = "<latest>"

# Core dependencies — resolve each version from crates.io via `cargo search`
anyhow = "<latest>"
axum = { version = "<latest>", default-features = false }
bytes = "<latest>"
serde = { version = "<latest>", features = ["derive"] }
serde_json = "<latest>"
tracing = "<latest>"
wasip3 = { version = "<latest>", features = ["http-compat"] }
wit-bindgen = { version = "<latest>", features = ["async-spawn"] }

# Native-only (runtime + test harness)
# wasmtime version must match the version used by omnia — check omnia's Cargo.toml
wasmtime = { version = "<latest>", default-features = false, features = ["component-model-async", "parallel-compilation"] }
wasmtime-wasi = { version = "<latest>", features = ["p3"] }
```

### Package Configuration

```toml
[package]
name = "<guest-name>"
description = "WASM guest for <service description>"
# ... use workspace inheritance for common fields

[lib]
crate-type = ["cdylib"]

[[example]]
name = "<guest-name>"
path = "examples/<guest-name>.rs"
```

### Release Profile

Optimize for WASM size and performance:

```toml
[profile.release]
lto = "thin"
opt-level = "s"
strip = "symbols"
```

### Guest Package Dependencies

The guest package adds wasm32-compatible dependencies. Note that `axum` must enable features for JSON, macros, and query extraction:

```toml
[dependencies]
anyhow.workspace = true
axum = { workspace = true, features = ["json", "macros", "query"] }
bytes.workspace = true
tracing.workspace = true
omnia-sdk.workspace = true
omnia-wasi-http.workspace = true
omnia-wasi-otel.workspace = true
wasip3.workspace = true
# Domain crates
<domain-crate> = { path = "crates/<domain-crate>" }
# Add only if needed:
# omnia-wasi-messaging.workspace = true    # If messaging used
# omnia-wasi-keyvalue.workspace = true     # If StateStore used

[dev-dependencies]
# Dependencies that work on all targets
cfg-if = "<latest>"
serde_json.workspace = true

[target.'cfg(not(target_arch = "wasm32"))'.dev-dependencies]
# Native-only dependencies for local runtime / integration tests
omnia.workspace = true
omnia-wasi-config.workspace = true
wasmtime.workspace = true
wasmtime-wasi.workspace = true
```

The `cfg-if` crate is needed for the runtime example's conditional compilation. This split ensures `cargo build` (targeting `wasm32-wasip2`) does not pull in native-only crates.

### Workspace Lints

Configure workspace-level lints for consistency:

```toml
[workspace.lints.rust]
trivial_numeric_casts = "warn"
unused_extern_crates = "warn"
unsafe_op_in_unsafe_fn = "warn"

[workspace.lints.clippy]
# Lint groups
all = "warn"      # correctness, suspicious, style, complexity, perf
nursery = "warn"
pedantic = "warn"
cargo = "warn"

# Cherry-picked restriction lints
# See https://microsoft.github.io/rust-guidelines/guidelines/universal/index.html
as_pointer_underscore = "warn"
assertions_on_result_states = "warn"
clone_on_ref_ptr = "warn"
deref_by_slicing = "warn"
disallowed_script_idents = "warn"
empty_drop = "warn"
empty_enum_variants_with_brackets = "warn"
empty_structs_with_brackets = "warn"
fn_to_numeric_cast_any = "warn"
if_then_some_else_none = "warn"
map_err_ignore = "warn"
redundant_type_annotations = "warn"
renamed_function_params = "warn"
semicolon_outside_block = "warn"
undocumented_unsafe_blocks = "warn"
unnecessary_safety_comment = "warn"
unnecessary_safety_doc = "warn"
unneeded_field_pattern = "warn"
unused_result_ok = "warn"
```

Then reference in the package:

```toml
[lints]
workspace = true
```

### Core Dependencies

| Dependency             | Purpose                                                  |
| ---------------------- | -------------------------------------------------------- |
| `anyhow`               | Error context and propagation                            |
| `axum`                 | HTTP routing (enable `json`, `macros`, `query` features) |
| `bytes`                | Efficient byte buffer for HTTP body extraction           |
| `omnia-sdk`            | SDK types, traits, and macros                            |
| `omnia-wasi-http`      | HTTP server/client support                               |
| `omnia-wasi-messaging` | Message pub/sub                                          |
| `omnia-wasi-otel`      | OpenTelemetry instrumentation                            |
| `tracing`              | Structured logging                                       |
| `wasip3`               | WASI P3 HTTP exports                                     |
| `wit-bindgen`          | WIT binding generation                                   |

---

## Config Templates

Copy-paste templates for standard project configuration files. These files are identical across all WASM guest projects.

### rustfmt.toml

```toml
# https://github.com/rust-lang/rustfmt/blob/master/Configurations.md
# https://rust-lang.github.io/rustfmt

max_width = 100
use_small_heuristics = "Max"

fn_params_layout = "Compressed"
format_code_in_doc_comments = true
format_macro_matchers = true
group_imports = "StdExternalCrate"
imports_granularity = "Module"
reorder_impl_items = true
unstable_features = true
use_field_init_shorthand = true
```

Requires `channel = "nightly"` in `rust-toolchain.toml` for the unstable formatting options.

### rust-toolchain.toml

```toml
[toolchain]
channel = "nightly"
components = ["clippy", "rust-src", "rustfmt"]
targets = [
  "wasm32-wasip2",
]
```

- `nightly` is required for unstable rustfmt options and `edition = "2024"`
- `rust-src` is needed for rust-analyzer to resolve WASI standard library types
- `wasm32-wasip2` is the WASM Component Model target

### .vscode/settings.json

```json
{
  "rust-analyzer.linkedProjects": ["Cargo.toml"],
  "rust-analyzer.check.command": "clippy",
  "rust-analyzer.cargo.cfgs": ["!miri"],
  "rust-analyzer.cargo.target": "wasm32-wasip2"
}
```

Configures rust-analyzer to target `wasm32-wasip2` so IDE diagnostics match the build target.

### clippy.toml

```toml
# https://doc.rust-lang.org/stable/clippy/index.html

doc-valid-idents = [
    # Add project-specific identifiers here (e.g., "TomTom", "OpenLR")
]

allowed-duplicate-crates = [
    # Populated by running `cargo clippy` and adding false positives.
    # Common entries for Omnia guests:
    "core-foundation",
    "embedded-io",
    "foldhash",
    "getrandom",
    "hashbrown",
    "linux-raw-sys",
    "rand",
    "rand_chacha",
    "rand_core",
    "reqwest",
    "rustix",
    "rustls",
    "rustls-webpki",
    "thiserror",
    "thiserror-impl",
    "tokio-rustls",
    "wasm-encoder",
    "wasm-metadata",
    "wasmparser",
    "wasi",
    "webpki-roots",
    "windows-link",
    "windows-result",
    "windows-strings",
    "windows-sys",
    "windows-targets",
    "windows_aarch64_gnullvm",
    "windows_aarch64_msvc",
    "windows_i686_gnu",
    "windows_i686_gnullvm",
    "windows_i686_msvc",
    "windows_x86_64_gnu",
    "windows_x86_64_gnullvm",
    "windows_x86_64_msvc",
    "wit-bindgen",
    "wit-bindgen-core",
    "wit-bindgen-rust",
    "wit-bindgen-rust-macro",
    "wit-component",
    "wit-parser",
]
```

- `doc-valid-idents` -- add domain-specific identifiers that appear in doc comments (prevents `doc_markdown` lint)
- `allowed-duplicate-crates` -- suppress false-positive duplicate crate warnings from transitive dependencies. Run `cargo clippy` after adding dependencies and update this list as needed.

### Makefile.toml

Standard `cargo-make` task runner. Install with `cargo install cargo-make`.

```toml
# Install: `cargo install cargo-make`
# Help: https://sagiegurari.github.io/cargo-make/

[env]
CARGO_MAKE_EXTEND_WORKSPACE_MAKEFILE = true

[config]
default_to_workspace = true
skip_core_tasks = true
skip_crate_env_info = true
skip_git_env_info = true
skip_rust_env_info = true

# -------------------------------------
# CI Checks
# -------------------------------------
[tasks.check]
dependencies = ["audit", "fmt", "lint", "outdated", "deps"]

[tasks.ci]
dependencies = ["lint", "test", "test-doc", "vet", "outdated", "deny", "fmt"]

# -------------------------------------
# Individual Actions
# -------------------------------------

# Audit
[tasks.audit]
command = "cargo"
args = ["audit"]

# Clean
[tasks.clean]
command = "cargo"
args = ["clean"]

# Deny
[tasks.deny]
command = "cargo"
args = ["deny", "--workspace", "check"]
install_crate = "cargo-deny"

# Deps
[tasks.deps]
script = '''
cargo +nightly udeps --all-targets
'''
install_crate = "cargo-udeps"

# Fmt
[tasks.fmt]
script = "cargo +nightly fmt --all"

# Lint
[tasks.lint]
command = "cargo"
args = ["clippy", "--all-features"]
install_crate = { rustup_component_name = "clippy" }

# Miri
[tasks.miri]
script = '''
cargo +nightly miri setup
cargo +nightly miri nextest run --no-tests=pass
'''

# Outdated
[tasks.outdated]
script = '''
cargo outdated --workspace --exit-code 1 --depth 1
'''
install_crate = "cargo-outdated"

# Test
[tasks.test]
command = "cargo"
args = ["nextest", "run", "--all", "--all-features", "--no-tests=pass"]
dependencies = ["clean"]
env = { RUSTFLAGS = "-Dwarnings" }

# Test Doc
[tasks.test-doc]
command = "cargo"
args = ["test", "--doc", "--all-features", "--workspace"]

# Vet
[tasks.vet]
script = '''
cargo vet regenerate imports
cargo vet regenerate exemptions
cargo vet regenerate unpublished
cargo vet --locked
'''
install_crate = "cargo-vet"

[tasks.publish]
command = "cargo"
args = ["publish", "--allow-dirty", "--dry-run"]
```

### deny.toml

Configuration for [`cargo-deny`](https://embarkstudios.github.io/cargo-deny/). This template is standard across Omnia guests.

```toml
# https://embarkstudios.github.io/cargo-deny/checks/licenses/cfg.html

[graph]
targets = [
  "aarch64-apple-darwin",
  "wasm32-wasip2",
]
all-features = true

[output]
feature-depth = 1

[advisories]
unmaintained = "transitive"

[licenses]
allow = [
  "Apache-2.0",
  "Apache-2.0 WITH LLVM-exception",
  "BSD-2-Clause",
  "BSD-3-Clause",
  "CDLA-Permissive-2.0",
  "ISC",
  "MIT",
  "OpenSSL",
  "Unicode-3.0",
  "Zlib",
]

[licenses.private]
ignore = true

[[licenses.clarify]]
name = "ring"
expression = "MIT AND ISC AND OpenSSL"
license-files = [{ path = "LICENSE", hash = 0xbd0eed23 }]

[bans]
multiple-versions = "allow"
wildcards = "allow"
deny = [
  { name = "tokio", deny-multiple-versions = true },
]
skip-tree = [
  { crate = "wasip2", depth = 5, reason = "contains out of date versions" },
]

[bans.workspace-dependencies]
duplicates = "deny"
include-path-dependencies = true
unused = "warn"

[sources]
```

**Customization notes**:

- `[graph].targets` -- the two standard targets for Omnia guests. Add additional targets if needed.
- `[licenses].allow` -- standard permissive license allowlist. Extend if your dependencies require additional licenses.
- `[bans].skip-tree` -- `wasip2` skip is needed due to outdated transitive versions in the WASI ecosystem.
- `[sources]` -- add private registry URLs here if the project uses any private registries.

### supply-chain/

Directory for [`cargo-vet`](https://mozilla.github.io/cargo-vet/) supply-chain security files. The skill generates scaffold files with static content (imports, README), then `cargo vet` commands populate the workspace-specific data (exemptions, policies, audits, imports.lock).

#### supply-chain/README.md

````markdown
# Cargo Vet

Following a Cargo dependency update, run:

```bash
cargo vet regenerate imports
cargo vet regenerate exemptions
cargo vet regenerate unpublished
```

to update the vetted dependencies based on trusted authors.

See the [Cargo Vet book](https://mozilla.github.io/cargo-vet/commands.html) for
more information.
````

#### supply-chain/config.toml

The `[imports]` section references trusted external audit sources used across the WASM/Wasmtime ecosystem. These are standard for all Omnia guests.

```toml

# cargo-vet config file

[cargo-vet]
version = "0.10"

[imports.bytecode-alliance]
url = "https://raw.githubusercontent.com/bytecodealliance/wasmtime/main/supply-chain/audits.toml"

[imports.embark-studios]
url = "https://raw.githubusercontent.com/EmbarkStudios/rust-ecosystem/main/audits.toml"

[imports.google]
url = "https://raw.githubusercontent.com/google/supply-chain/main/audits.toml"

[imports.isrg]
url = "https://raw.githubusercontent.com/divviup/libprio-rs/main/supply-chain/audits.toml"

[imports.mozilla]
url = "https://raw.githubusercontent.com/mozilla/supply-chain/main/audits.toml"

[imports.zcash]
url = "https://raw.githubusercontent.com/zcash/rust-ecosystem/main/supply-chain/audits.toml"

# [policy.<crate-name>] and [[exemptions.<crate-name>]] entries are
# workspace-specific. They are populated by running:
#
#   cargo vet regenerate exemptions
#
# after workspace dependencies are finalized.
```

**Import sources**:

| Import              | Source                   | Covers                               |
| ------------------- | ------------------------ | ------------------------------------ |
| `bytecode-alliance` | Wasmtime supply chain    | Wasmtime, Cranelift, WASI crates     |
| `embark-studios`    | Embark Studios ecosystem | General Rust ecosystem crates        |
| `google`            | Google supply chain      | General Rust ecosystem crates        |
| `isrg`              | ISRG / libprio-rs        | Cryptography-adjacent crates         |
| `mozilla`           | Mozilla supply chain     | Firefox/Servo Rust ecosystem crates  |
| `zcash`             | Zcash ecosystem          | Cryptography and general Rust crates |

#### supply-chain/audits.toml

Minimal scaffold. Trusted publisher entries are populated by `cargo vet` commands.

```toml

# cargo-vet audits file

[audits]
```

#### Post-Generation: Populate Workspace-Specific Data

After all project files are generated and workspace dependencies are finalized (i.e., `Cargo.toml` and `Cargo.lock` exist), run the following commands to populate exemptions, policies, trusted publishers, and import data:

```bash
cargo vet regenerate imports
cargo vet regenerate exemptions
cargo vet regenerate unpublished
```

These commands will:

- **`regenerate imports`** -- fetch audit data from the 6 import sources and write `supply-chain/imports.lock`
- **`regenerate exemptions`** -- add `[[exemptions.<crate>]]` entries to `supply-chain/config.toml` for any dependency not covered by imports or audits
- **`regenerate unpublished`** -- add `[policy.<crate>]` entries for workspace crates with `audit-as-crates-io = true`

**Note**: `supply-chain/imports.lock` is auto-generated by `cargo vet regenerate imports` and should NOT be hand-written or templated.

### Config File Reference

| File                        | Purpose                                | Template                       |
| --------------------------- | -------------------------------------- | ------------------------------ |
| `rustfmt.toml`              | Nightly formatting config              | Standard (above)               |
| `rust-toolchain.toml`       | Nightly channel + wasm32 target        | Standard (above)               |
| `.vscode/settings.json`     | rust-analyzer wasm32 config            | Standard (above)               |
| `clippy.toml`               | Lint exceptions                        | Customize per project          |
| `Makefile.toml`             | CI/dev task runner                     | Standard (above)               |
| `deny.toml`                 | Dependency license/advisory/ban checks | Standard (above)               |
| `supply-chain/README.md`    | Cargo Vet update instructions          | Standard (above)               |
| `supply-chain/config.toml`  | Cargo Vet imports + scaffold           | Standard (above) + `cargo vet` |
| `supply-chain/audits.toml`  | Cargo Vet trusted publishers           | Scaffold (above) + `cargo vet` |
| `supply-chain/imports.lock` | Imported audit data                    | Auto-generated by `cargo vet`  |

---

## GitHub Workflows

Standard GitHub Actions workflow files for WASM guest projects. All workflows delegate to reusable workflows in the `augentic/.github` repository.

Every guest project includes 5 workflow files in `.github/workflows/`:

| File           | Trigger                 | Purpose                                    |
| -------------- | ----------------------- | ------------------------------------------ |
| `audit.yaml`   | Daily schedule + manual | Security audit of dependencies             |
| `ci.yaml`      | Push to any branch      | Continuous integration (build, lint, test) |
| `patch.yaml`   | Manual                  | Create a patch release                     |
| `release.yaml` | Manual                  | Create a new release                       |
| `publish.yaml` | Manual                  | Full pipeline: CI -> Publish -> Deploy     |

### Required Secrets

Configure these secrets in the GitHub repository settings:

| Secret                   | Used by | Purpose                                     |
| ------------------------ | ------- | ------------------------------------------- |
| `AZURE_CLIENT_ID`        | publish | Azure service principal for WASM deployment |
| `AZURE_TENANT_ID`        | publish | Azure tenant for WASM deployment            |
| `AZURE_SUBSCRIPTION_ID`  | publish | Azure subscription for WASM deployment      |

### audit.yaml

Runs a daily security audit of dependencies. Can also be triggered manually.

```yaml
name: Audit

on:
  schedule:
    - cron: "0 0 * * *"
  workflow_dispatch:

jobs:
  audit:
    uses: augentic/.github/.github/workflows/audit.yaml@main
```

### ci.yaml

Runs on every push. Delegates to the shared CI workflow which builds, lints, and tests the project.

```yaml
name: CI

on:
  push:

jobs:
  ci:
    uses: augentic/.github/.github/workflows/ci.yaml@main
    secrets:
      CARGO_REGISTRY_TOKEN: ${{ secrets.CARGO_REGISTRY_TOKEN }}
```

### patch.yaml

Manually triggered to create a patch release.

```yaml
name: Create Patch

on:
  workflow_dispatch:

jobs:
  patch:
    uses: augentic/.github/.github/workflows/patch.yaml@main
```

### release.yaml

Manually triggered to create a new release.

```yaml
name: Create Release

on:
  workflow_dispatch:

jobs:
  release:
    uses: augentic/.github/.github/workflows/release.yaml@main
```

### publish.yaml

Manually triggered three-stage pipeline: CI, Publish, then Deploy. The deploy stage pushes the compiled WASM component to Azure.

Replace the placeholder values in the `with:` block:

| Parameter         | Description                                                       |
| ----------------- | ----------------------------------------------------------------- |
| `package`         | Crate name of the guest (e.g. the package name from `Cargo.toml`) |
| `storage-account` | Azure Storage account for WASM deployment                         |
| `resource-group`  | Azure resource group for WASM deployment                          |

```yaml
name: Publish Release

on:
  workflow_dispatch:

jobs:
  ci:
    name: CI
    uses: augentic/.github/.github/workflows/ci.yaml@main

  publish:
    name: Publish
    needs: ci
    uses: augentic/.github/.github/workflows/publish.yaml@main

  deploy:
    name: Deploy
    needs: publish
    uses: augentic/.github/.github/workflows/wasm.yaml@main
    secrets:
      AZURE_CLIENT_ID: ${{ secrets.AZURE_CLIENT_ID }}
      AZURE_TENANT_ID: ${{ secrets.AZURE_TENANT_ID }}
      AZURE_SUBSCRIPTION_ID: ${{ secrets.AZURE_SUBSCRIPTION_ID }}
    with:
      package: <PACKAGE_NAME>
      storage-account: <STORAGE_ACCOUNT>
      resource-group: <RESOURCE_GROUP>
```
