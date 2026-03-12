# Subtractive Example: Remove an Endpoint from the Cars Crate

Removing the `GET /feature/{id}` endpoint from the `cars` multi-handler crate because the feature lookup is no longer in the updated artifacts.

## Starting State

The `cars` multi-handler crate as shown in the [crate-writer multi-handler example](../multi-handler.md):

```
crates/cars/
├── Cargo.toml
├── src/
│   ├── lib.rs
│   ├── filter.rs
│   ├── handlers.rs             # barrel: feature, feature_list, layout, worksite
│   └── handlers/
│       ├── feature.rs          # GET /feature/{id} -- TO BE REMOVED
│       ├── feature_list.rs     # GET /features
│       ├── layout.rs           # GET /layout
│       └── worksite.rs         # GET /worksite
├── tests/
│   ├── provider.rs
│   ├── feature.rs              # Tests for feature endpoint -- TO BE REMOVED
│   ├── layout.rs
│   └── worksite.rs
├── tests/data/
│   └── feature_response.json   # Fixture for feature tests -- TO BE REMOVED
├── Migration.md
├── Architecture.md
└── .env.example
```

## Artifact Change

The updated artifacts no longer contain the `GET /feature/{id}` endpoint in the API Contracts section. All other endpoints remain.

The types `FeatureRequest`, `FeatureResponse`, `MwsFeature`, and `MwsProperties` are used ONLY by the feature handler.

## Derived Change Set

- **Category**: Subtractive
- **Changes**:
  1. Delete `src/handlers/feature.rs`
  2. Remove module declaration and re-export from `src/handlers.rs`
  3. Delete `tests/feature.rs`
  4. Delete `tests/data/feature_response.json` (if no other tests use it)
  5. Remove guest wiring (route, import, handler function)
  6. Document in CHANGELOG.md

## Pre-Removal Safety Check

Before removing, verify:

- [x] `FeatureRequest` is NOT referenced by any other handler (grep confirms only `feature.rs` uses it)
- [x] `FeatureResponse` is NOT referenced by any other handler
- [x] `MwsFeature` is NOT referenced by any other handler
- [x] `MwsProperties` is NOT referenced by any other handler
- [x] The endpoint is truly absent from the new artifacts (searched all sections)

## Applied Changes

### 1. Delete Handler File

Delete `src/handlers/feature.rs` entirely. This removes:

- `FeatureRequest` struct and its `Handler<P>` impl
- `FeatureResponse` struct and its `IntoBody` impl
- `MwsFeature` and `MwsProperties` types
- The standalone `handle` function
- The `From<Worksite> for MwsFeature` conversion

### 2. Update Barrel (`src/handlers.rs`)

**Before**:

```rust
mod feature;
mod feature_list;
mod layout;
mod worksite;

pub use feature::*;
pub use feature_list::*;
pub use layout::*;
pub use worksite::*;
```

**After**:

```rust
mod feature_list;
mod layout;
mod worksite;

pub use feature_list::*;
pub use layout::*;
pub use worksite::*;
```

### 3. Delete Test File

Delete `tests/feature.rs` entirely.

### 4. Delete Test Fixture

Delete `tests/data/feature_response.json` if no other test references it. Verify:

```bash
grep -r "feature_response" tests/ --include="*.rs"
```

If other tests reference it, leave it in place.

### 5. Update Guest Wiring (`$PROJECT_DIR/src/lib.rs`)

Remove the route:

```rust
// REMOVE:
.route("/feature/{id}", get(feature_handler))
```

Remove the import:

```rust
// REMOVE:
use cars::{FeatureRequest, FeatureResponse};
```

Remove the handler function:

```rust
// REMOVE this entire function:
#[omnia_wasi_otel::instrument]
async fn feature_handler(Path(id): Path<String>) -> HttpResult<Reply<FeatureResponse>> {
    FeatureRequest::handler(id)?
        .provider(&Provider::new())
        .owner("at")
        .await
        .map_err(Into::into)
}
```

### 6. Check for Orphaned Dependencies

After removal, check if any dependencies in `Cargo.toml` are now unused. In this case, `serde_json` is still used by other handlers, so no dependency changes are needed.

If `feature.rs` had unique dependencies (e.g., a crate only it imported), remove those from `Cargo.toml`.

### 7. CHANGELOG.md Entry

```markdown
## [Update: 2026-03-01]

### Removed
- Removed `GET /feature/{id}` endpoint (no longer in artifacts)
- Removed types: `FeatureRequest`, `FeatureResponse`, `MwsFeature`, `MwsProperties`
- Removed `From<Worksite> for MwsFeature` conversion
```

## What Was NOT Changed

- `src/lib.rs` -- no module-level changes needed (barrel handles it)
- `src/filter.rs` -- still used by worksite and layout handlers
- `src/handlers/feature_list.rs` -- separate handler, unaffected
- `src/handlers/layout.rs` -- unrelated handler
- `src/handlers/worksite.rs` -- unrelated handler
- `tests/provider.rs` -- MockProvider still needed by other tests
- `Cargo.toml` -- all dependencies still in use
- `.env.example` -- `MWS_API_KEY` still used by other handlers

## Verification

- [x] Baseline `cargo test` captured (all existing tests pass)
- [x] Removed handler file, test file, and fixture
- [x] Barrel module updated (no orphaned `mod` or `pub use`)
- [x] No remaining references to `FeatureRequest`, `FeatureResponse`, `MwsFeature`, `MwsProperties`
- [x] Guest wiring updated: route, import, and handler function removed
- [x] No orphaned dependencies in `Cargo.toml`
- [x] No regressions: all remaining tests still pass
- [x] `cargo check` passes
- [x] `cargo clippy` passes
- [x] CHANGELOG.md documents the removal with reason
