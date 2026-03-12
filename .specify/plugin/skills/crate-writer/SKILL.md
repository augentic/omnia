---
name: crate-writer
description: "Write Rust WASM crates from Specify artifacts -- greenfield creation or incremental updates -- following Omnia SDK patterns with provider-based dependency injection."
argument-hint: [crate-name] [project-dir?] [--skip-tests?] [change-description?]
allowed-tools: Read, Write, StrReplace, Shell, Grep, ReadLints
user-invocable: true
context: fork
agent: general-purpose
---

# Crate Writer

Write Rust WASM crates from Specify artifacts (specs + design.md), following Omnia SDK patterns for stateless, provider-based WASM components. This skill handles both **greenfield creation** and **incremental updates** to existing crates.

**Mode detection**: If `$CRATE_PATH/Cargo.toml` exists, the skill runs in **update mode** (surgical changes preserving existing behavior). Otherwise it runs in **create mode** (full crate generation from scratch).

This skill accepts Specify artifacts from any producer:

- **Requirements artifacts** (from `epic-analyzer`) -- generates/updates crates from JIRA epics, including enriched design
- **Code-Analysis artifacts** (from `code-analyzer`) -- generates/updates crates from existing source code
- **Feature specs** (from Specify change artifacts) -- updated specs derived from requirements changes

## Authority Hierarchy

When conflicts arise, follow this strict precedence:

1. **This SKILL.md** (highest) -- generation/update rules and hard constraints
2. **Specify artifacts (specs + design.md)** -- behavior specification (artifacts always win for changed behavior)
3. **references/** -- authoritative patterns and SDK API
4. **examples/** -- canonical production code patterns
5. **Existing crate code** (UPDATE MODE ONLY) -- authoritative for unchanged behavior; trust existing code for anything the updated artifacts do not contradict
6. **Original source** (if provided) -- reference for ambiguity only
7. **LLM inference** (lowest) -- prohibited for `[unknown]` cases; use TODO markers

**Key difference between modes**: In update mode, existing crate code sits at level 5 -- authoritative for any behavior the artifacts do not explicitly change. In create mode, there is no existing code; levels 5-6 are skipped.

## Hard Rules

Violations of any rule below fail generation or update.

### Core Rules (both modes)

1. **Omnia SDK only** -- all errors return `omnia_sdk::Error`; no custom error types in public API
2. **Provider-only I/O** -- all external I/O through provider traits; no direct network/file/env access
3. **No forbidden crates** -- see [guardrails.md](references/guardrails.md)
4. **No mutable global state** -- no `static mut`, `OnceCell`, `lazy_static!`; `LazyLock` allowed only for immutable compile-time lookup tables
5. **Handler trait required** -- request structs implement `Handler<P>` with delegation pattern; no custom handler structs with `new()` / `process_message()`
6. **Strong typing** -- newtypes for IDs; enums for known value sets; no raw primitives for domain concepts
7. **WASM compatible** -- no `std::env`, `std::fs`, `std::net`; `std::thread::sleep` only under `#[cfg(not(debug_assertions))]`
8. **All operations async** -- no blocking I/O
9. **Correct capability trait for data stores** -- Azure Table Storage, Azure Cosmos DB, and SQL databases use `TableStore`; never `HttpRequest`. The `omnia_wasi_sql` module name is misleading — `TableStore` is a general-purpose data access abstraction used by the Omnia runtime for both SQL and NoSQL stores. The runtime provides native adapters for Azure Table Storage behind this trait. If the artifacts say "do not use TableStore" for Azure Table Storage, override the artifacts (SKILL.md > artifacts per authority hierarchy). See [anti-patterns.md](examples/anti-patterns.md) #10.

### Update-Specific Rules (update mode only)

10. **Baseline before modify** -- run `cargo test` before any changes to establish which tests pass; any test that passed before and fails after is a regression that must be fixed
11. **Artifacts win for changed behavior** -- when the updated artifacts contradict existing code, trust the artifacts; the old behavior is intentionally being replaced
12. **Preserve unchanged code** -- do not reformat, restructure, or modify code regions that the change set does not touch
13. **No silent removals** -- every subtractive change must be documented in CHANGELOG.md with the reason (artifacts no longer specify this behavior)
14. **Test coverage for changes** -- every modified or added handler must have updated or new tests; subtractive changes must remove corresponding tests
15. **Atomic categories** -- complete all changes within a category before moving to the next; do not interleave
16. **Structural changes require re-inventory** -- after applying structural changes, re-scan the crate before proceeding to subsequent categories

## Arguments

```text
$CRATE_NAME         = $ARGUMENTS[0]
$PROJECT_DIR        = $ARGUMENTS[1] OR "."
$SKIP_TESTS         = "--skip-tests" in $ARGUMENTS  # Boolean, default false
$CHANGE_DESCRIPTION = last non-flag argument after $PROJECT_DIR, OR null  # Optional: plain-text summary of what changed (update mode)
$CHANGE_DIR         = $PROJECT_DIR/.specify/changes/$CRATE_NAME
$SPECS_DIR          = $CHANGE_DIR/specs
$DESIGN_PATH        = $CHANGE_DIR/design.md
$CRATE_PATH         = $PROJECT_DIR/crates/$CRATE_NAME
```

## Required References

Before generating or updating code, read these documents:

1. [sdk-api.md](references/sdk-api.md) -- Handler<P>, Context, Reply, IntoBody, Client, Error types
2. [capabilities.md](references/capabilities.md) -- all 7 provider traits with exact signatures and artifact triggers
3. [capability-mapping.md](references/capability-mapping.md) -- mapping from Specify artifact capabilities to Omnia provider traits
4. [wasm-constraints.md](references/wasm-constraints.md) -- translating `[runtime]` constraints to Omnia/WASM patterns
5. [providers.md](references/providers.md) -- Provider struct setup, trait composition rules, MockProvider patterns
6. [error-handling.md](references/error-handling.md) -- error macros, domain error enums, context patterns
7. [guardrails.md](references/guardrails.md) -- WASM constraints and forbidden patterns
8. [cargo-toml.md](references/cargo-toml.md) -- Cargo.toml template and dependency rules
9. [guest-wiring.md](references/guest-wiring.md) -- how crates wire into the WASM guest

**Both modes** -- also read:

10. [checklists.md](references/checklists.md) -- pre-generation and verification checklists
11. [todo-markers.md](references/todo-markers.md) -- TODO marker rules, capability overrides, cache-aside patterns
12. [output-documents.md](references/output-documents.md) -- Migration.md, Architecture.md, CHANGELOG.md, .env.example

**Update mode only** -- also read:

13. [update-patterns.md](references/update-patterns.md) -- update strategy patterns by category
14. [change-classification.md](references/change-classification.md) -- how to classify artifact-vs-code differences

### Examples

**Create mode** (read at least one matching your scenario):

- [single-handler.md](examples/single-handler.md) -- messaging handler crate (like r9k-adapter)
- [multi-handler.md](examples/multi-handler.md) -- multiple HTTP handlers crate (like cars)
- [testing.md](examples/testing.md) -- MockProvider and test patterns
- [anti-patterns.md](examples/anti-patterns.md) -- common LLM mistakes with wrong/right pairs
- [capabilities/](examples/capabilities/) -- per-capability worked examples (StateStore, Identity, TableStore, Broadcast, etc.)

**Update mode** (read at least one matching your update scenario):

- [updates/additive.md](examples/updates/additive.md) -- add a new handler to an existing crate
- [updates/modifying.md](examples/updates/modifying.md) -- change business logic in an existing handler
- [updates/subtractive.md](examples/updates/subtractive.md) -- remove an endpoint and its handler
- [updates/structural.md](examples/updates/structural.md) -- refactor a domain model

## Artifact Dispatch

Read design.md Context section to determine origin:

```markdown
## Context

- **Source**: <jira-epic | source-code>
```

### Requirements Artifact Mapping (Source: jira-epic)

- **design.md Domain Model > Entities** -> `src/types.rs`
- **design.md API Contracts > Endpoints** -> `src/handlers.rs`
- **design.md Business Logic** -> domain modules or inline in handler
- **design.md Source Capabilities Summary + design.md External Services** -> handler trait bounds (via [capability-mapping.md](references/capability-mapping.md))
- **specs/ Requirements** -> traceability comments (`// Source: JIRA $STORY_KEY`)
- **`[infrastructure]` steps without Omnia equivalent** -> TODO comment in handler with suggested Omnia approach; documented in Migration.md

### Code-Analysis Artifact Mapping (Source: source-code)

- **design.md Domain Model > Types** -> `src/types.rs` (preserve exact nesting)
- **design.md API Contracts > API Calls** -> `src/handlers.rs`
- **design.md Business Logic** -> domain modules or inline in handler
- **design.md External Services + Source Capabilities Summary + Business Logic cues** -> handler trait bounds (via [capability-mapping.md](references/capability-mapping.md))
- **design.md Implementation Requirements `[runtime]` constraints** -> Omnia patterns (via [wasm-constraints.md](references/wasm-constraints.md))
- **Source paths** -> reference comments (`// Source: $SOURCE_PATH`)
- **`[infrastructure]` steps without Omnia equivalent** -> TODO comment in handler with suggested Omnia approach; documented in Migration.md

## Crate Structure

### Single Handler (messaging adapter, connector)

```
$CRATE_PATH/
├── Cargo.toml
├── src/
│   ├── lib.rs              # Module declarations, error types, re-exports
│   ├── handler.rs          # Handler<P> impl + standalone handle() fn
│   ├── <input_domain>.rs   # Input types (deserialization, validation)
│   └── <output_domain>.rs  # Output types (serialization)
├── tests/
│   ├── provider.rs         # MockProvider implementing required traits
│   └── <test_name>.rs      # Integration tests using Client
├── Migration.md
├── Architecture.md
└── .env.example
```

See [single-handler.md](examples/single-handler.md) for the r9k-adapter example.

### Multi Handler (API crate with multiple endpoints)

**Layout**: Prefer **Multi** handler modules when there are many endpoints/handlers or when handlers share substantial types in the barrel. Use a **barrel + directory** (`src/handlers.rs` + `src/handlers/*.rs`). Flat layout is more idiomatic for small crates and keeps discovery simple.

```
# Flat (preferred for small crates)
$CRATE_PATH/src/
├── lib.rs              # mod r9k; mod smartrak; pub use r9k::*; pub use smartrak::*;
├── r9k.rs   # Handler<P> impl
├── smartrak.rs
└── types.rs            # Shared types

# Barrel + directory (valid for larger crates)
$CRATE_PATH/src/
├── lib.rs
├── handlers.rs         # Barrel + shared types
├── handlers/
│   ├── <endpoint_a>.rs
│   └── <endpoint_b>.rs
└── <utility>.rs
```

See [multi-handler.md](examples/multi-handler.md) for the barrel layout example.

## Handler Pattern

Every handler follows this exact pattern. The request struct implements `Handler<P>` and delegates to a standalone async function.

```rust
use omnia_sdk::api::{Context, Handler, Reply};
use omnia_sdk::{Config, Error, HttpRequest, Result};

async fn handle<P>(_owner: &str, request: MyRequest, provider: &P) -> Result<Reply<MyResponse>>
where
    P: Config + HttpRequest,
{
    // business logic here
    Ok(Reply::ok(response))
}

impl<P> Handler<P> for MyRequest
where
    P: Config + HttpRequest,
{
    type Error = Error;
    type Input = Vec<u8>;
    type Output = MyResponse;

    fn from_input(input: Vec<u8>) -> Result<Self> {
        serde_json::from_slice(&input)
            .context("deserializing MyRequest")
            .map_err(Into::into)
    }

    async fn handle(self, ctx: Context<'_, P>) -> Result<Reply<MyResponse>> {
        handle(ctx.owner, self, ctx.provider).await
    }
}
```

### Input Type Decision Tree

| Scenario                             | `type Input`       | `from_input` pattern                                     |
| ------------------------------------ | ------------------ | -------------------------------------------------------- |
| Message/POST body                    | `Vec<u8>`          | `serde_json::from_slice` or `quick_xml::de::from_reader` |
| Single path param (`GET /item/{id}`) | `String`           | `Ok(Self { id: input })`                                 |
| Tuple path params (`GET /a/{x}/{y}`) | `(String, String)` | `Ok(Self { x: input.0, y: input.1 })`                    |
| Query string (`GET /search?q=...`)   | `Option<String>`   | `serde_urlencoded::from_str(&input.unwrap_or_default())` |
| Scheduled/cron (no payload)          | `()`               | `Ok(Self)`                                               |

**Never** use `type Input = MyRequest` -- this bypasses deserialization and is incompatible with the Omnia runtime.

### Response Types

HTTP response types must implement `IntoBody` (see [sdk-api.md](references/sdk-api.md) for the full API). Messaging handlers that produce no HTTP response use `type Output = ()`.

## Error Handling

Domain errors use `thiserror` and convert to `omnia_sdk::Error` via `From<DomainError>`. Use error macros for one-off errors: `bad_request!("msg")`, `server_error!("msg")`, `bad_gateway!("msg")`.

See [error-handling.md](references/error-handling.md) for domain error patterns, macro usage, and context chaining.

### Validation Placement

**Rule 1: Structural validation belongs in `from_input()`**

Checks that depend ONLY on the parsed data structure:

- Field presence: `if field.is_empty()`
- Format validation: regex, email, phone number
- Range checks on constants: `if value < MIN || value > MAX`
- Type conversions that can fail: parsing enums, dates with fixed formats

**Rule 2: Temporal/contextual validation belongs in `handle()` or a `validate()` method**

Checks that compare against runtime state or time:

- Timestamp freshness: `if delay_secs > MAX_DELAY`
- Message age checks using `Utc::now()`
- Idempotency checks (requires StateStore lookup)
- Business rule validation (requires external API calls)

**Decision flowchart**:

```text
Does the validation use Utc::now(), SystemTime, or runtime state?
├─ YES → belongs in handle() or validate() method called from handle()
└─ NO → can this check be done immediately after parsing?
   ├─ YES → belongs in from_input()
   └─ NO → belongs in handle()
```

Never use `Utc::now()` in `from_input()` -- test framework's `shift_time` cannot fix validation at parse time.

### Timestamp, DST, and Serde Rules

- **`received_at`** -- always `Utc::now()` (processing time), never source creation timestamp. SKILL.md > artifacts on this.
- **DST-safe conversion** -- use `.earliest()` not `.single()` for local timezone conversion.
- **Serde directionality** -- input-only types: `#[serde(rename(deserialize = "..."))]`; output types: `#[serde(rename_all = "camelCase")]`.

See [error-handling.md](references/error-handling.md) for detailed timestamp semantics, DST patterns, and serde conventions.

## Test Generation

**Skip when `$SKIP_TESTS` is true.** In that case, skip test generation and the Tests items in the verification checklist.

Generate baseline tests alongside the crate using `test-writer` patterns. crate-writer produces happy path + error case tests inline. For comprehensive test suites, spec-to-test compilation, or replay-pattern tests with time-shifting, use `test-writer` or `replay-writer` respectively.

See [testing.md](examples/testing.md) for complete patterns and [mock-provider.md](references/mock-provider.md) for MockProvider implementations.

## Guest Wiring (Conditional)

**Trigger**: only when `$PROJECT_DIR/src/lib.rs` exists.

After generating or updating the crate, inject or update wiring in the guest project. See [guest-wiring.md](references/guest-wiring.md) for templates.

### What to Inject (create mode)

1. `use $crate_name::{...};` import for handler types
2. Axum route entries (HTTP handlers)
3. Topic match arms (messaging handlers)
4. WebSocket handler delegation (WebSocket handlers) -- add delegation inside existing WebSocket Guest impl, or create the full WebSocket Guest export block if none exists
5. Handler functions with `#[omnia_wasi_otel::instrument]`
6. Provider trait impls if new capabilities needed
7. Crate dependency in `$PROJECT_DIR/Cargo.toml`

### Guest Wiring by Category (update mode)

| Category        | Guest Wiring Action                                     |
| --------------- | ------------------------------------------------------- |
| **Additive**    | Append new routes/topics/imports (append-only pattern)  |
| **Subtractive** | Remove routes/topics/imports for deleted handlers       |
| **Modifying**   | Update route paths, HTTP methods, or handler signatures |
| **Structural**  | Update import names after type/module renames            |

### Rules (both modes)

- Append only in create mode -- do not replace or reorder existing content
- No duplicates -- skip if route/topic/WebSocket handler/import already exists
- All handler functions get `#[omnia_wasi_otel::instrument]`
- Update Provider trait impls if capabilities changed
- Update `ensure_env!` entries for config key changes

## Pre-Generation Checklist

Before starting code generation, verify artifact completeness per [checklists.md](references/checklists.md#pre-generation-checklist). If ANY item is NO or UNCLEAR, mark with TODO in generated code and note in Migration.md.

---

## Mode: Create

**When**: `$CRATE_PATH/Cargo.toml` does NOT exist.

### Generation Process

1. Read Specify artifacts from `$CHANGE_DIR`:
   - Read the spec file from `$SPECS_DIR/$CRATE_NAME/spec.md` (single consolidated file with `## Handler:` sections)
   - Read design.md from `$DESIGN_PATH`
2. **Derive Omnia capabilities from artifacts:**
   - Read the design.md **Source Capabilities Summary** checklist and map each checked capability to an Omnia provider trait using [capability-mapping.md](references/capability-mapping.md).
   - Read the design.md **External Services** and cross-reference service types against the mapping table. Verify that managed table stores (Azure Table Storage, Cosmos DB) map to `TableStore`, databases map to `TableStore`, caches map to `StateStore`, etc.
   - Read the design.md **Implementation Requirements** `[runtime]` constraints and translate each to an Omnia pattern using [wasm-constraints.md](references/wasm-constraints.md).
   - Scan design.md **Business Logic** for data access phrasing (`Table access:`, `Cache:`) and map to appropriate traits.
3. **Artifact correction — fix known misassignments before generating** (SKILL.md > artifacts per authority hierarchy):
   - If design.md External Services lists a `managed table store` but the Source Capabilities Summary does not check `Table/database access`: **add `TableStore`** to the derived traits.
   - If any algorithm step phrases managed table store access as an HTTP call: **override to `TableStore`**.
   - If the artifacts describe pre-populating a cache via external cron/ETL for data the source loads on startup: **override to on-demand cache-aside** (StateStore + data source trait).
4. Determine artifact origin from design.md Context section
5. Read reference documents from `references/`
6. Read matching example from `examples/`
7. Run pre-generation checklist above (verify artifact completeness)
8. Generate `Cargo.toml` (see [cargo-toml.md](references/cargo-toml.md))
9. Generate `src/lib.rs` with module declarations and re-exports
10. Generate `src/types.rs` or domain type modules
11. Generate `src/handlers.rs` (or `src/handler.rs` for single handler)
12. Generate domain-specific modules as needed
13. (skip if `$SKIP_TESTS`) Generate `tests/provider.rs` with MockProvider
14. (skip if `$SKIP_TESTS`) Generate `tests/<name>.rs` with integration tests
15. Generate `Migration.md`, `Architecture.md`, `.env.example`
16. Run `cargo check` to verify compilation
17. Run verification checklist
18. If `$PROJECT_DIR/src/lib.rs` exists: inject guest wiring (see Guest Wiring section above)

---

## Mode: Update

**When**: `$CRATE_PATH/Cargo.toml` exists.

### Update Scope

Four categories of change, ordered by application priority:

| Category        | Description                                                                | Examples                                                                                                            | Complexity |
| --------------- | -------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------- | ---------- |
| **Structural**  | Changes to domain model relationships, type renames, handler splits/merges | Rename `OrderEvent` to `PurchaseEvent` across all files; split a multi-handler module; merge two handlers into one  | High       |
| **Subtractive** | Removal of handlers, endpoints, types, or features                         | Remove a deprecated endpoint; delete unused types; remove a topic handler                                           | Medium     |
| **Modifying**   | Changes to existing business logic, validation, types, or provider bounds  | Add a field to an existing type; change validation threshold; add a new provider trait bound; update error handling | Medium     |
| **Additive**    | New handlers, endpoints, types, or features added to an existing crate     | Add a new HTTP handler; add a new domain type; add a new test                                                       | Low        |

### Application Order

Changes are applied in this fixed order to minimize intermediate breakage:

1. **Structural** -- type renames and relationship changes propagate to all downstream code
2. **Subtractive** -- remove dead code before adding or modifying to avoid conflicts
3. **Modifying** -- update existing implementations with stable references in place
4. **Additive** -- new code depends on the already-updated type system

### Update Process

#### Step 0: Read References

Read all documents listed in [Required References](#required-references) including update-specific references, and at least one matching update example.

#### Step 1: Capture Baseline

Before any changes, establish the current state:

```bash
cd $CRATE_PATH && cargo test 2>&1 | tee $PROJECT_DIR/temp/$CRATE_NAME-baseline.txt
```

Record which tests pass and which fail. This baseline is used in Step 7 to detect regressions.

If `cargo test` fails to compile, record the compilation errors -- the update must not introduce additional compilation failures.

#### Step 2: Inventory Existing Crate

Parse the existing crate to build a structural inventory mapping artifact concepts to file locations:

| Source                                                      | What to Extract                                                                               |
| ----------------------------------------------------------- | --------------------------------------------------------------------------------------------- |
| `Cargo.toml`                                                | Crate name, dependencies, features                                                            |
| `src/lib.rs`                                                | Module declarations, re-exports, error type definitions                                       |
| `src/handler.rs` or `src/handlers.rs` + `src/handlers/*.rs` | Handler implementations, provider trait bounds, input types, `from_input` patterns            |
| `src/types.rs` and domain modules                           | Type definitions, serde attributes, newtypes, enums                                           |
| `tests/provider.rs`                                         | MockProvider: which traits are implemented, what config keys return, what HTTP fixtures exist |
| `tests/*.rs`                                                | Test names, what handlers they cover, assertion patterns                                      |
| `$PROJECT_DIR/src/lib.rs` (guest, if exists)                | Routes, topic arms, WebSocket handlers, imports, Provider trait impls                         |

The inventory is an in-memory working model, not a persisted artifact. For each item, record:

- **Concept**: handler name, type name, endpoint path, topic, etc.
- **File**: path relative to `$CRATE_PATH`
- **Lines**: approximate line range
- **Signature**: handler trait bounds, type fields, serde attributes

#### Step 3: Derive Change Set

Read the updated artifacts from `$CHANGE_DIR` (specs and design.md) and compare them against the inventory:

| Artifacts vs Inventory                                                             | Classification  |
| ---------------------------------------------------------------------------------- | --------------- |
| Handler/endpoint in artifacts but not in inventory                                 | **Additive**    |
| Handler in both, but business logic, input/output types, or provider bounds differ | **Modifying**   |
| Handler/endpoint in inventory but not in artifacts                                 | **Subtractive** |
| Type renamed, relationships changed, handler split/merged                          | **Structural**  |
| Type in artifacts but not in inventory                                              | **Additive**    |
| Type in both but fields/attributes differ                                          | **Modifying**   |
| Type in inventory but not in artifacts                                              | **Subtractive** |
| Config key in artifacts but not in `.env.example`                                  | **Additive**    |
| Config key in `.env.example` but not in artifacts                                  | **Subtractive** |

If `$CHANGE_DESCRIPTION` is provided, use it to:

- **Focus**: prioritize changes that match the description
- **Validate**: confirm the derived change set is consistent with the stated intent
- **Flag**: warn if the derived change set includes unexpected changes not mentioned in the description

See [change-classification.md](references/change-classification.md) for detailed classification rules and edge cases.

#### Step 4: Generate Update Plan

For each change, determine the specific edit operations. The plan is a structured list:

```text
STRUCTURAL (apply first):
  1. Rename OrderEvent → PurchaseEvent
     - src/types.rs: lines 15-30 (struct definition)
     - src/handler.rs: lines 45, 67 (references)
     - tests/handler_test.rs: lines 12, 34 (test fixtures)

SUBTRACTIVE (apply second):
  2. Remove GET /legacy-status endpoint
     - src/handlers/legacy_status.rs: delete file
     - src/handlers.rs: remove mod + pub use
     - tests/legacy_status.rs: delete file
     - guest src/lib.rs: remove route + import

MODIFYING (apply third):
  3. Add `priority` field to WorksiteRequest
     - src/handlers/worksite.rs: lines 20-28 (struct definition)
     - src/handlers/worksite.rs: lines 45-60 (filter builder)
     - tests/worksite.rs: lines 15-30 (test fixture)

ADDITIVE (apply last):
  4. Add POST /worksite handler
     - src/handlers/create_worksite.rs: new file
     - src/handlers.rs: add mod + pub use
     - tests/create_worksite.rs: new file
     - guest src/lib.rs: add route + import
```

Log the plan for traceability. Do not modify any files in this step.

#### Step 5: Apply Changes by Category

Execute the plan in the fixed order: structural, subtractive, modifying, additive.

**Structural Changes**: Rename types, modules, or restructure relationships. After completing all structural changes:
- Run `cargo check` to verify compilation
- Re-scan the crate to update the inventory (Hard Rule 16)
- Proceed only if compilation passes
- Patterns: See [update-patterns.md](references/update-patterns.md#structural-patterns)

**Subtractive Changes**: Remove handlers, types, tests, and guest wiring for features no longer in the artifacts:
1. Remove handler implementation files (or handler functions from shared files)
2. Remove corresponding type definitions (only if not used by remaining handlers)
3. Remove corresponding test files
4. Remove module declarations from `lib.rs` or barrel modules
5. Remove unused dependencies from `Cargo.toml`
6. Document each removal in CHANGELOG.md (Hard Rule 13)
- Patterns: See [update-patterns.md](references/update-patterns.md#subtractive-patterns)

**Modifying Changes**: Update existing handler logic, types, or provider bounds:
1. Update type definitions (fields, serde attributes, derive macros)
2. Update handler business logic to match updated artifacts
3. Update provider trait bounds if new capabilities are needed
4. Update `from_input()` for structural validation changes
5. Update `handle()` for temporal/contextual validation changes
6. Update MockProvider in `tests/provider.rs` for new capabilities
7. Update existing tests to match new behavior
8. Preserve function signatures where possible; when signatures change, update all call sites
- Patterns: See [update-patterns.md](references/update-patterns.md#modifying-patterns)

**Additive Changes**: Add new handlers, types, and tests following the create-mode patterns exactly:
1. Generate new handler files following the Handler pattern
2. Generate new type definitions
3. Add module declarations to `lib.rs` or barrel modules
4. Add dependencies to `Cargo.toml`
5. Generate new test files
6. Update MockProvider if new provider traits are needed
- Patterns: See [update-patterns.md](references/update-patterns.md#additive-patterns)

#### Step 6: Update Guest Wiring

If `$PROJECT_DIR/src/lib.rs` exists, apply guest wiring changes per the Guest Wiring by Category table above.

#### Step 7: Verify (repair loop -- max 3 iterations)

Run the verification suite as a bounded repair loop. Each iteration runs all
three checks; if any fail, fix the issue and start a new iteration.

**7a. Formatting**:
```bash
cd $CRATE_PATH && cargo fmt --check
```
If fails: run `cargo fmt` to fix. Formatting is mechanical; one pass suffices.

**7b. Compilation and lint**:
```bash
cd $CRATE_PATH && cargo check
cd $CRATE_PATH && cargo clippy -- -D warnings
```
If fails: fix each warning using [repair-patterns.md](references/repair-patterns.md)
(clippy section), then re-run.

**7c. Test suite**:
```bash
cd $CRATE_PATH && cargo test 2>&1 | tee $PROJECT_DIR/temp/$CRATE_NAME-post-update.txt
```

Compare against the baseline from Step 1:
- Tests that passed before must still pass (no regressions)
- New tests for added/modified handlers must pass
- Tests for removed handlers should no longer exist

If failures are detected, apply the matching repair strategy from
[repair-patterns.md](references/repair-patterns.md):
- Type mismatches: update struct definitions, serde attributes
- Validation errors: fix `from_input()` or `handle()` logic
- Provider mock errors: update MockProvider to match new trait bounds
- Logic errors: compare handler implementation against artifacts

**Loop control**: Repeat from 7a until all three pass or 3 iterations are
exhausted. If still failing after 3 iterations: STOP. Do not mark the task
complete. Report the remaining failures with error output and escalate for
guidance.

**7d. Verification Checklist**: Run the full checklist from the Verification Checklist section below.

---

## TODO Markers

Any functionality that cannot be fully implemented must be marked with a TODO at the call site AND documented in Migration.md. Never silently drop artifact steps. See [todo-markers.md](references/todo-markers.md) for the full marker format, capability override rules, managed data store recognition, TableStore/StateStore inference, and cache-aside patterns.

## Verification Checklist

Before completing, verify ALL items per [checklists.md](references/checklists.md#verification-checklist). This covers compilation, handler compliance, artifact fidelity, type quality, tests, guest wiring, and update-mode-specific checks.

## Output Documents

Generate documentation artifacts per [output-documents.md](references/output-documents.md): Migration.md, Architecture.md, CHANGELOG.md (update mode), and .env.example.

## Troubleshooting

See [error-handling.md](references/error-handling.md#troubleshooting) for common issues and resolutions in both create and update modes.

## Output Hygiene

Only emit: `.rs` source files, `Cargo.toml`, and the required docs. Do not emit `target/`, `Cargo.lock`, or build artifacts.

## Important Notes

- **Mode is auto-detected**: If `$CRATE_PATH/Cargo.toml` exists, update mode runs. Otherwise, create mode runs.
- In update mode, existing tests are treated as specifications: a previously-passing test that fails after update is a regression, not an expected change.
- In update mode, changes are applied in fixed order (structural → subtractive → modifying → additive) regardless of `$CHANGE_DESCRIPTION`.
- In update mode, after structural changes, the crate is re-inventoried before subsequent categories to ensure references are current.
- When in doubt about whether a change is required (update mode), compare the specific artifact section against the existing code. If they match, no change is needed.
