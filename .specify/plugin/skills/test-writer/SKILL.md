---
name: test-writer
description: "Generate or update test suites for Omnia Rust WASM crates from Specify artifacts -- MockProvider setup, integration tests, spec-to-test mapping, and drift detection."
argument-hint: [crate-name] [project-dir?]
allowed-tools: Read, Write, StrReplace, Shell, Grep, ReadLints
user-invocable: true
---

# Test Writer

Generate or update test suites for Omnia Rust WASM crates from Specify artifacts (specs + design.md) and existing crate code. Tests use `MockProvider` implementations and the `Client` typestate builder to invoke handlers.

**Relationship to other skills**:

- **crate-writer** generates baseline tests (happy path + error cases) alongside the crate. test-writer provides comprehensive test generation, spec-to-test traceability, and standalone test updates.
- **replay-writer** adds regression tests from captured real-world fixtures. test-writer generates synthetic tests from spec scenarios.

Use test-writer when:

- You need comprehensive tests beyond crate-writer's baseline coverage
- Specs have changed and tests need updating to match
- You want spec-to-test traceability (each BDD scenario maps to a test)
- You want to detect drift between specs and existing tests

## Arguments

```text
$CRATE_NAME   = $ARGUMENTS[0]
$PROJECT_DIR  = $ARGUMENTS[1] OR "."
$CRATE_PATH   = $PROJECT_DIR/crates/$CRATE_NAME
$CHANGE_DIR   = $PROJECT_DIR/.specify/changes/$CRATE_NAME
$SPECS_DIR    = $CHANGE_DIR/specs
$DESIGN_PATH  = $CHANGE_DIR/design.md
```

## Required References

Before generating tests, read these documents:

1. [mock-provider.md](references/mock-provider.md) -- Static and Replay MockProvider patterns
2. [spec-to-test-mapping.md](references/spec-to-test-mapping.md) -- How spec scenarios map to test functions

### Examples

Read at least one matching your scenario:

- [testing.md](examples/testing.md) -- Core test patterns: layout, MockProvider, test structures, fixtures
- [testing-http.md](examples/testing-http.md) -- Simple HTTP handler testing with Config-only MockProvider
- [testing-statestore.md](examples/testing-statestore.md) -- Multi-trait MockProvider with StateStore and cache-aside
- [testing-publisher.md](examples/testing-publisher.md) -- Publish, event capture, request-reply, topic checks

## Authority Hierarchy

When conflicts arise, follow this strict precedence:

1. **This SKILL.md** -- test generation rules
2. **Specify artifacts (specs + design.md)** -- behavioral requirements that tests must verify
3. **references/** -- MockProvider and mapping patterns
4. **examples/** -- canonical test code patterns
5. **Existing crate code** -- handler signatures, provider bounds, type definitions
6. **Existing tests** -- style and conventions to follow

## Test Generation Process

### Step 1: Read Crate and Artifacts

1. Read the spec file from `$SPECS_DIR/$CRATE_NAME/spec.md` (consolidated file with `## Handler:` sections)
2. Read design.md from `$DESIGN_PATH`
3. Read existing crate code from `$CRATE_PATH/src/` to identify:
   - Handler implementations and their provider trait bounds
   - Input/output types and serde attributes
   - Domain error variants
   - Validation logic (structural in `from_input()`, temporal in `handle()`)

### Step 2: Inventory Existing Tests

If `$CRATE_PATH/tests/` exists, parse it to understand the current test state:

| Source | What to Extract |
| --- | --- |
| `tests/provider.rs` | MockProvider: which traits implemented, config keys, HTTP fixtures |
| `tests/*.rs` | Test names, handlers covered, assertion patterns, fixture usage |
| `tests/data/` | Existing fixture files |

### Step 3: Map Spec Scenarios to Tests

For each `## Handler:` section in the spec, and each `#### Scenario:` or `##### Scenario:` within it:

1. **One test function per scenario** -- deterministic naming: `test_<handler>_<scenario_snake_case>`
2. **Happy path tests** from success scenarios (WHEN/THEN with expected output)
3. **Error case tests** from error scenarios (WHEN/THEN with expected error code)
4. **Validation tests** from requirement constraints (field presence, format, range)

See [spec-to-test-mapping.md](references/spec-to-test-mapping.md) for the detailed mapping rules.

### Step 4: Generate MockProvider

Generate `tests/provider.rs` implementing all provider traits the handlers require:

- **Config**: Return test values for each `Config::get` key in the crate; error for unknown keys
- **HttpRequest**: Dispatch on `request.uri().path()` to return fixture data; record requests for assertion
- **Publish**: Capture events via `Arc<Mutex<Vec<T>>>` for assertion
- **Identity**: Return mock tokens
- **StateStore**: In-memory `HashMap` behind `Mutex` with get/set/delete
- **TableStore**: Return fixture rows from `query`, affected count from `exec`
- **Broadcast**: Capture sends with channel and target info

See [mock-provider.md](references/mock-provider.md) for complete patterns (Static and Replay variants).

### Step 5: Generate Test Files

For each handler, generate a test file at `tests/<handler_name>.rs`:

```rust
mod provider;

use <crate_name>::<HandlerRequest>;
use omnia_sdk::api::Client;
use provider::MockProvider;

#[tokio::test]
async fn test_<handler>_happy_path() {
    let provider = MockProvider::new();
    let client = Client::new("owner").provider(provider.clone());

    let request = <HandlerRequest> { /* fields from scenario */ };
    let response = client.request(request).await.expect("should succeed");

    assert_eq!(response.status, 200);
    // assert on response.body fields per scenario THEN clause
}

#[tokio::test]
async fn test_<handler>_<error_scenario>() {
    let provider = MockProvider::new();
    let client = Client::new("owner").provider(provider.clone());

    let request = <HandlerRequest> { /* fields triggering error */ };
    let error = client.request(request).await.expect_err("should fail");

    assert_eq!(error.code(), "<expected_code>");
}
```

### Step 6: Generate Fixture Data

For tests that require mock HTTP responses or complex input data:

- Store JSON fixtures in `tests/data/` (e.g., `tests/data/worksite-search.json`)
- Reference in MockProvider with `include_bytes!("data/<fixture>.json")`
- Derive fixture content from design.md API response shapes and example data

### Step 7: Verify

```bash
cd $CRATE_PATH && cargo test
```

All generated tests must pass. If failures occur:

1. Check MockProvider trait implementations match handler bounds
2. Verify fixture data shapes match expected API response types
3. Confirm assertion values align with spec scenario THEN clauses
4. Fix and re-run until all tests pass

## Test Conventions

1. **Each test file** starts with `mod provider;`
2. **Create provider** with `MockProvider::new()`
3. **Create client** with `Client::new("owner").provider(provider.clone())`
4. **Invoke handler** with `client.request(request).await`
5. **Assert on response**: `response.status`, `response.body`
6. **Assert on side effects**: `provider.events()`, `provider.requests_for(path)`
7. **Error testing**: `.expect_err("message")` then assert `error.code()` and `error.description()`
8. **Async runtime**: `#[tokio::test]`
9. **tokio in dev-dependencies only**: `tokio = { version = "1", features = ["macros", "rt"] }`

## Test Directory Structure

```
$CRATE_PATH/
├── tests/
│   ├── provider.rs         # MockProvider (shared across tests)
│   ├── <handler_a>.rs      # Tests for handler A
│   └── <handler_b>.rs      # Tests for handler B
└── tests/data/             # JSON/XML fixture files (optional)
    ├── response-a.json
    └── response-b.json
```

## Spec-to-Test Mapping (Forward-Looking)

The long-term goal is deterministic, repeatable spec-to-test compilation:

- **Each BDD scenario in spec.md maps to exactly one test function**. The mapping is deterministic -- the same spec always produces the same test structure.
- **Spec drift detection**: Regenerate tests from baseline specs at `.specify/specs/$CRATE_NAME/spec.md` and compare against existing tests. Differences indicate either spec drift (spec changed without updating tests) or code drift (code changed without updating spec).
- **CI integration**: A future CI step can regenerate tests from specs, diff against committed tests, and fail the build if they diverge. This closes the loop: specs produce tests, tests validate code, and drift is caught automatically.

See [spec-to-test-mapping.md](references/spec-to-test-mapping.md) for the mapping rules that enable this.

## Drift Detection (Forward-Looking)

When invoked against a crate with existing tests and baseline specs:

1. **Regenerate** the expected test structure from `.specify/specs/$CRATE_NAME/spec.md`
2. **Compare** against existing tests in `$CRATE_PATH/tests/`
3. **Report** divergences:
   - **Missing tests**: Spec scenarios with no corresponding test function
   - **Extra tests**: Test functions with no corresponding spec scenario (may be manual additions -- flag, don't remove)
   - **Assertion drift**: Test assertions that don't match spec THEN clauses
4. **Surface** as either spec drift or code drift for human review

This enables the spec-as-contract model: specs have teeth because tests enforce them, and drift is visible.

## Verification Checklist

Before completing, verify ALL items:

- [ ] `tests/provider.rs` with MockProvider implementing all required traits
- [ ] At least one test per handler (happy path)
- [ ] Error case tests for validation failures documented in specs
- [ ] Tests use `Client::new("owner").provider(mock)` pattern
- [ ] Test fixtures in `tests/data/` or inline
- [ ] `cargo test` passes
- [ ] No `unwrap()` or `expect()` in production code (allowed in tests)
- [ ] Each spec scenario has a corresponding test function (when specs are available)

## Related Skills

- **crate-writer** -- generates crates with optional baseline tests; delegates comprehensive testing here
- **replay-writer** -- adds regression tests from captured real-world fixtures (complementary; test-writer generates from specs, replay-writer generates from production data)
- **code-reviewer** -- reviews generated code including test quality
