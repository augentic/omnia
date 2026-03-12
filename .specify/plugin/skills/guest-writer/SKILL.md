---
name: guest-writer
description: Generate a Rust project that exposes HTTP endpoints, subscribes to message topics, and handles WebSocket events in order to surface business logic via the Omnia WASI runtime.
argument-hint: [project-dir?]
allowed-tools: Read, Write, StrReplace, Shell
user-invocable: false
context: fork
agent: general-purpose
---

# Guest Generator Skill

## Overview

Generate a complete WASM guest project that wraps one or more domain crates containing business logic. The guest provides the wiring layer that:

- Exposes HTTP endpoints using `wasip3::http` types
- Handles message subscription using `omnia_wasi_messaging` types
- Handles WebSocket events using `omnia_wasi_websocket` types
- Configures provider traits for WASI capabilities (Config, Publish, Identity, StateStore, TableStore, etc.)
- Bridges domain logic to the Omnia WASI runtime

## Key Principle

The guest is a thin wrapper. It handles WASI/wasm32 boundary concerns such as HTTP routing, subscribing to message topics, handling WebSocket events, and provider setup. **ALL** business logic is delegated to project crates (in the crates/ directory).

## ⚠️ IMPORTANT: Consult reference documentation before commencing

**Before building the guest project:**

1. **Read the relevant reference docs** for the features you're implementing:
   - HTTP endpoints, messaging, and WebSocket → [Handlers](references/handlers.md)
   - Provider and runtime → [Providers](references/omnia/providers/README.md), [Runtime](references/omnia/runtime.md)
   - Dependencies, config templates, and CI/CD → [Configuration](references/configuration.md)
   - Guest macro (optional) → [guest! Macro](references/handlers.md#guest-macro)

2. **Verify all constraints and patterns** in the reference docs match your generated code

3. **Use the reference docs as the source of truth** for:
   - Correct dependency versions and registry configuration
   - Export macros and trait implementations
   - Error handling patterns
   - Configuration validation patterns

**Reference docs are NOT examples. They are specifications.** Please follow them exactly.

## Derived Arguments

1. **Project directory** (`$PROJECT_DIR`): Directory for the WASM guest project. If not provided, the current directory should be used. Default value `.`

```text
$PROJECT_DIR   = $ARGUMENTS[0] OR "."
```

## Process

### Step 1: Generate project structure

Create the guest project at `$PROJECT_DIR` with structure:

```text
$PROJECT_DIR/
├── .cargo/config.toml   # Registry + credential providers
├── .github/
│   └── workflows/
│       ├── audit.yaml   # Daily security audit
│       ├── ci.yaml      # CI on every push
│       ├── patch.yaml   # Create patch release
│       ├── publish.yaml # CI → Publish → Deploy pipeline
│       └── release.yaml # Create release
├── .vscode/settings.json # rust-analyzer wasm32 config
├── Cargo.toml           # Workspace and dependencies
├── Makefile.toml        # Build tasks (cargo-make)
├── clippy.toml          # Lint exceptions
├── deny.toml            # Dependency checks
├── rust-toolchain.toml  # Nightly + wasm32 target
├── rustfmt.toml         # Formatting rules
├── src/
│   └── lib.rs           # HTTP and Messaging Guest implementations
├── supply-chain/
│   └── audits.toml      # Cargo vet audits file
│   └── config.toml      # Cargo vet config file
│   └── imports.lock     # Cargo vet lock file
│   └── README.md        # Cargo vet instruction file
├── examples/
│   ├── <guest>.rs       # Local runtime via omnia::runtime!
│   └── .env.example     # Environment template
└── crates/              # (optional) local crates
```

See [Project structure](references/project.md) for complete layout and [Configuration](references/configuration.md) for standard config file contents.

### Step 2: Generate Cargo.toml

All omnia packages are published to the Credibil and at-realtime registries. **Configure `.cargo/config.toml` first** -- see [Configuration](references/configuration.md) for the full configuration including registry URLs, credential providers, and net settings.

Then configure workspace dependencies based on domain crate requirements:

- Always include: `omnia-sdk`, `anyhow`, `bytes`, `tracing`
- If HTTP used: `omnia-wasi-http`, `axum` (with features `["json", "macros", "query"]`)
- If messaging used: `omnia-wasi-messaging`
- If WebSocket used: `omnia-wasi-websocket`
- If StateStore used: `omnia-wasi-keyvalue`
- If Identity used: `omnia-wasi-identity`
- If TableStore used: `omnia-wasi-sql`

All `omnia-*` crates are published on **crates.io**. No private registry configuration is needed.

See [Configuration](references/configuration.md) for dependency patterns and version resolution instructions.

### Step 3: Generate src/lib.rs

Generate the main guest module. Choose one approach:

**Option A: Manual wiring** -- full control over HTTP routing, messaging dispatch, WebSocket handling, and handler invocation:

1. **HTTP Guest** -- Axum router with routes using `{param}` syntax (Axum 0.8)
2. **Messaging Guest** -- Topic dispatcher that returns `Err` for unhandled topics
3. **WebSocket Guest** -- Event handler that delegates to domain crate handlers
4. **Handler invocation** -- use the builder API: `Type::handler(input)?.provider(&provider).owner("owner").await`
5. **Provider** -- trait implementations for WASI capabilities

**Option B: `guest!` macro** -- declarative DSL that generates all wiring. See [Handlers](references/handlers.md#guest-macro) for syntax and the [Complete lib.rs Example](references/handlers.md#complete-librs-example) for both approaches side by side.

**Owner**: every handler requires an `owner` string identifying the Omnia component owner (e.g. `"at"`). See [Providers](references/omnia/providers/README.md#owner) for details.

See also [Handlers](references/handlers.md) for HTTP, messaging, and WebSocket patterns.

### Step 4: Generate Runtime Example

Create `examples/<guest>.rs` with `omnia::runtime!` macro to enable local development and testing.

See [Runtime Setup](references/omnia/runtime.md).

### Step 5: Generate Environment Template

Create `examples/.env.example` with all required config keys documented.

### Step 6: Generate GitHub Workflows

Create `.github/workflows/` with the standard CI/CD workflow files. All workflows delegate to reusable workflows in the `augentic/.github` repository.

Generate all 5 files: `audit.yaml`, `ci.yaml`, `patch.yaml`, `publish.yaml`, `release.yaml`.

For `publish.yaml`, configure the project-specific deployment parameters (`package`, `storage-account`, `resource-group`) based on the project context.

See [Configuration](references/configuration.md#github-workflows) for templates and required secrets.

### Step 7: Generate Supply-Chain and Compliance Files

Generate dependency compliance configuration for Cargo Deny and Cargo Vet:

1. **`deny.toml`** -- Cargo Deny configuration for license, advisory, ban, and source checks.
   Use the standard template from [Configuration](references/configuration.md#denytom).
   Customize `[sources].private` to match the project's private registry URL(s) from `.cargo/config.toml`.

2. **`supply-chain/README.md`** -- Instructions for updating vetted dependencies after code changes.

3. **`supply-chain/config.toml`** -- Cargo Vet configuration with standard imports from trusted audit sources (bytecode-alliance, embark-studios, google, isrg, mozilla, zcash).

4. **`supply-chain/audits.toml`** -- Empty scaffold (populated by cargo vet commands).

5. **`supply-chain/imports.lock`** --- Empty lock file (populated by cargo vet commands).

After generating all project files, run:

```bash
cargo vet regenerate imports
cargo vet regenerate exemptions
cargo vet regenerate unpublished
```

These commands populate workspace-specific exemptions, policies, trusted publishers, and import data in the supply-chain directory. They require `Cargo.toml` and `Cargo.lock` to exist.

See [Configuration](references/configuration.md) for all templates and post-generation details.

## Reference Documentation

Detailed guidance and specifications are available in `references/`:

- **[Project structure](references/project.md)** - Directory layout and file organization
- **[Handlers](references/handlers.md)** - HTTP routing, message subscriptions, WebSocket events, lib.rs wiring, and `guest!` macro
- **[Providers](references/omnia/providers/README.md)** - WASI capability provider patterns
- **[Runtime](references/omnia/runtime.md)** - Local development runtime setup
- **[Configuration](references/configuration.md)** - Cargo workspace, config templates, and GitHub workflows

## Examples

Refer to the crate-specific examples that demonstrate guest wiring patterns:

- [Guest Wiring Pattern](references/omnia/guest-wiring.md) - How to wire HTTP routes, messaging topics, and WebSocket events into the guest project
- [Runtime Setup](references/omnia/runtime.md) - Runtime example generation pattern

Each example includes the expected directory structure, generated files, and key wiring patterns.

## Error Handling

### Common Issues and Resolutions

| Issue                        | Cause                                  | Resolution                                              |
| ---------------------------- | -------------------------------------- | ------------------------------------------------------- |
| `src/lib.rs` already exists  | Guest project previously generated     | Skip generation (idempotent check)                      |
| Missing route for endpoint   | Endpoint from domain crate not wired into Axum router | Check domain crate handler exports; add missing route to src/lib.rs |
| Missing messaging handler    | Topic subscription from domain crate not wired        | Check domain crate messaging handlers; add topic match arm          |
| Missing WebSocket handler    | WebSocket handler from domain crate not wired         | Check domain crate WebSocket exports; add handler delegation        |
| Provider missing trait impl  | New provider needed by domain crate    | Add trait implementation to Provider struct             |
| Cargo.toml dependency error  | Domain crate path incorrect            | Verify `$CRATE_PATH` relative to guest project root     |
| Build fails on wasm32 target | Non-WASM-compatible code in guest      | Check for std::env, std::fs, std::net usage             |

### Recovery Process

1. Run `cargo check` and capture errors
2. For missing routes: add endpoint to Axum router in `src/lib.rs`
3. For provider errors: add trait bounds and implementations
4. For build errors: verify wasm32 compatibility of all dependencies
5. Re-run `cargo check` after each fix

## Verification Checklist

Before completing, verify:

- [ ] All HTTP endpoints are routed in the Axum router
- [ ] All message subscriptions have handlers in MessagingGuest
- [ ] All WebSocket events have handlers in WebSocketGuest (if WebSocket is used)
- [ ] Provider implements all required traits
- [ ] All config keys are validated in Provider::new()
- [ ] Domain crate is properly imported in Cargo.toml
- [ ] Runtime example compiles with `cargo build --example`
- [ ] `.env.example` documents all required environment variables
- [ ] Config files present: `rustfmt.toml`, `rust-toolchain.toml`, `clippy.toml`, `.vscode/settings.json`, `Makefile.toml`
- [ ] GitHub workflows present: `audit.yaml`, `ci.yaml`, `patch.yaml`, `publish.yaml`, `release.yaml`
- [ ] `deny.toml` present with `[sources].private` matching `.cargo/config.toml` registry URL(s)
- [ ] `supply-chain/` directory present with `config.toml`, `audits.toml`, and `README.md`
- [ ] Handlers annotated with `#[omnia_wasi_otel::instrument]`

## Important Notes

### Hard Rules

1. **No business logic** -- All logic must live in domain crates
2. **wasm32 target only** -- `#![cfg(target_arch = "wasm32")]` guard required
3. **Provider-only I/O** -- All external i/o must be through provider traits
4. **Explicit topic routing** -- Messaging handler must match topics, return `Err` for unhandled
5. **WebSocket export** -- WebSocket handler must use `omnia_wasi_websocket::export!` and implement `omnia_wasi_websocket::incoming_handler::Guest`
6. **Owner required** -- Every handler invocation must include `.owner("...")` in the builder chain
7. **Builder API** -- Handler invocation uses `.provider(&p).owner("o").await`, not `.process(&p)`
8. **Route params** -- Use `{param}` brace syntax (Axum 0.8), not `:param`

### wasm32 Compatibility

- No `std::env`, `std::fs`, `std::net`, `std::thread`
- Config via `omnia_sdk::Config` trait
- Async only, no blocking operations
