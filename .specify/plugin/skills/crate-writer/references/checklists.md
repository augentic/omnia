# Checklists

Pre-generation and post-generation verification checklists for crate-writer.

## Pre-Generation Checklist

Before starting code generation, verify artifact completeness:

### Input Types

- [ ] All wire-format field names documented?
- [ ] Field optionality marked (yes/no/unknown)?
- [ ] Custom converters documented with behavior?

### Output Types

- [ ] Full schema documented (not just fields this component uses)?
- [ ] Nested structure preserved (not flattened)?
- [ ] Optional fields marked with `skip_serializing_if`?

### External APIs

- [ ] Response shapes show full nesting?
- [ ] Authentication flow documented?

### Business Logic

- [ ] Temporal validation separate from structural?
- [ ] Error model with exact variant count and codes?
- [ ] Every named external system call (`eventStore.put`, `keyVault.getSecret`, third-party HTTP, etc.) mapped to one of the 7 Omnia traits, OR flagged as TODO **in the code at the call site** (not just in Migration.md)?

### Required Capabilities

- [ ] design.md "Source Capabilities Summary" section, "External Services", and relevant Business Logic cues read in full?
- [ ] Capability-to-trait mapping applied per [capability-mapping.md](capability-mapping.md)?
- [ ] `[runtime]` constraints translated per [wasm-constraints.md](wasm-constraints.md)?
- [ ] Every named external system in business logic steps maps to one of the 7 Omnia traits, or flagged for TODO?
- [ ] Managed data store override applied? If design.md External Services lists a managed table store but algorithm steps phrase it as HTTP, override to `TableStore` per SKILL.md authority (see [todo-markers.md](todo-markers.md) "Capability override for managed data stores").

### Publication Patterns

- [ ] Publication count documented?
- [ ] Message metadata (keys, headers) documented?

**If ANY item is NO or UNCLEAR**: check if SKILL.md or references provide a default; otherwise mark with TODO in generated code and note in Migration.md. Do NOT guess.

---

## Verification Checklist

Before completing, verify ALL items.

### Compilation

- [ ] `cargo check` passes
- [ ] `cargo clippy` passes without warnings (where possible)
- [ ] No `println!`, `dbg!`, or `unsafe` code
- [ ] All dependencies use `workspace = true`
- [ ] `Cargo.toml`, `Migration.md`, `Architecture.md`, `.env.example` exist

### Handler Compliance

- [ ] Request structs implement `Handler<P>` (not custom handler structs)
- [ ] `handle()` delegates to standalone async function
- [ ] Provider bounds include ALL required traits
- [ ] Input types match the Input Type Decision Tree in SKILL.md
- [ ] HTTP response types implement `IntoBody`
- [ ] All errors return `omnia_sdk::Error`
- [ ] Domain errors implement `From<DomainError> for omnia_sdk::Error`

### Artifact Fidelity

- [ ] API response parsing matches artifact-documented shapes exactly
- [ ] Output nesting matches design.md (nested structs, not flattened)
- [ ] Config key names match artifacts verbatim
- [ ] All `[unknown]` items have TODO markers
- [ ] All `[infrastructure]` steps either map to Omnia traits or have TODO markers
- [ ] All `[domain]` steps that call a named external system either map to a provider trait or have TODO markers
- [ ] Every capability in design.md "Source Capabilities Summary" (and derived from External Services or Business Logic cues) is mapped to an Omnia trait, bound in `Handler<P>`, or has a TODO marker and Migration.md entry
- [ ] No managed data store accessed via `HttpRequest` — Azure Table Storage, Cosmos DB, Redis use `TableStore` or `StateStore` (see [todo-markers.md](todo-markers.md) "Capability override for managed data stores")
- [ ] No data-fetching logic deferred to assumed external cron/ETL — if legacy loads from a data store, the handler fetches on demand via cache-aside (see [todo-markers.md](todo-markers.md) "Startup cache → on-demand cache-aside")
- [ ] Every config key in the design.md Configuration section appears in `.env.example` — even keys whose implementation is a TODO
- [ ] No artifact algorithm steps silently dropped
- [ ] Every behavior omitted and documented in Migration.md has a corresponding TODO comment at the exact call site in the generated code

### Type Quality

- [ ] Public types derive `Clone, Debug, Serialize, Deserialize`
- [ ] Input-only types use `#[serde(rename(deserialize = "..."))]` not `#[serde(rename = "...")]`
- [ ] Input-only types have `#[serde(default)]` on struct
- [ ] Output types use `#[serde(rename_all = "camelCase")]`
- [ ] Optional fields use `#[serde(skip_serializing_if = "Option::is_none")]`
- [ ] Integer-backed enums use `serde_repr`
- [ ] All public items have doc comments

### Tests (skip when `$SKIP_TESTS`)

- [ ] `tests/provider.rs` with MockProvider implementing required traits
- [ ] At least one test per handler (happy path)
- [ ] Error case tests for validation failures
- [ ] Tests use `Client::new("owner").provider(mock)` pattern
- [ ] Test fixtures in `tests/data/` or inline

### Guest Wiring (when applicable)

- [ ] All HTTP endpoints registered in guest `src/lib.rs`
- [ ] All messaging topics registered in guest `src/lib.rs`
- [ ] All WebSocket handlers registered in guest `src/lib.rs`
- [ ] Handler types imported from the generated crate
- [ ] Guest `Cargo.toml` includes the new crate
- [ ] Provider trait implementations cover required capabilities
- [ ] No duplicate routes, topics, WebSocket handlers, or imports

### Update Mode Only

- [ ] Baseline `cargo test` captured before changes
- [ ] No regressions: all previously-passing tests still pass
- [ ] Post-update `cargo test` output saved to `$PROJECT_DIR/temp/$CRATE_NAME-post-update.txt`
- [ ] CHANGELOG.md entries for all changes (additive, modifying, subtractive, structural)
- [ ] Removed code has no orphaned references (unused imports, dead modules)
- [ ] Tests for removed handlers/endpoints deleted (Hard Rule 14)
- [ ] Modified handler signatures propagated to all call sites and tests
- [ ] `Architecture.md` updated to reflect structural changes
- [ ] `Migration.md` updated with new TODOs (if any)
