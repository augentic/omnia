# Structural Example: Refactor Domain Model in r9k-adapter

Renaming the `SmarTrakEvent` output type to `PositionEvent` and restructuring the module layout from `smartrak.rs` to `position.rs`. The artifacts reflect a rebranding of the downstream system's event schema.

## Starting State

The `r9k-adapter` single-handler crate:

```
crates/r9k-adapter/
├── Cargo.toml
├── src/
│   ├── lib.rs          # mod handler; mod r9k; mod smartrak; mod stops;
│   ├── handler.rs
│   ├── r9k.rs          # input types
│   ├── smartrak.rs     # output types: SmarTrakEvent, RemoteData, LocationData, etc.
│   └── stops.rs
├── tests/
│   ├── provider.rs
│   └── static.rs
├── Migration.md
├── Architecture.md
└── .env.example
```

## Artifact Change

The updated artifacts rename the output types in the Domain Model:

```markdown
### Domain Model > Entities

#### PositionEvent (was: SmarTrakEvent)
- `received_at`: DateTime<Utc>
- `event_type`: EventType
- `event_data`: EventData
- `message_data`: MessageData
- `remote_data`: RemoteData
- `location_data`: LocationData
- `company_id?`: u64
- `serial_data?`: SerialData

(All nested types RemoteData, LocationData, EventData, MessageData, SerialData unchanged)
```

The topic name also changes:

```markdown
### Constants & Configuration
- Publication topic: `realtime-r9k-to-position.v1` (was: `realtime-r9k-to-smartrak.v1`)
```

## Derived Change Set

- **Category**: Structural (type rename + module rename propagate across files)
- **Changes**:
  1. Rename `src/smartrak.rs` to `src/position.rs`
  2. Rename `SmarTrakEvent` to `PositionEvent` in all files
  3. Update module declaration in `src/lib.rs`
  4. Update topic constant in `src/handler.rs`
  5. Update all references in handler, stops, tests, and guest
  6. After structural changes complete, re-inventory before any further changes

## Applied Changes

### 1. Rename Module File

Rename `src/smartrak.rs` to `src/position.rs`. The file content remains the same except for the type rename.

### 2. Update Module Declaration (`src/lib.rs`)

**Before**:

```rust
mod handler;
mod r9k;
mod smartrak;
mod stops;

use omnia_sdk::Error;
use thiserror::Error;

pub use self::handler::*;
pub use self::r9k::*;
pub use self::smartrak::*;
pub use self::stops::StopInfo;
```

**After**:

```rust
mod handler;
mod position;
mod r9k;
mod stops;

use omnia_sdk::Error;
use thiserror::Error;

pub use self::handler::*;
pub use self::position::*;
pub use self::r9k::*;
pub use self::stops::StopInfo;
```

### 3. Rename Type in Output Module (`src/position.rs`)

**Before** (in former `smartrak.rs`):

```rust
//! SmarTrak event types.

// ...

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SmarTrakEvent {
    #[serde(serialize_with = "with_nanos")]
    pub received_at: DateTime<Utc>,
    pub event_type: EventType,
    pub event_data: EventData,
    pub message_data: MessageData,
    pub remote_data: RemoteData,
    pub location_data: LocationData,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub company_id: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub serial_data: Option<SerialData>,
}
```

**After** (`src/position.rs`):

```rust
//! Position event types.

// ...

/// A position event published to the realtime position topic.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PositionEvent {
    #[serde(serialize_with = "with_nanos")]
    pub received_at: DateTime<Utc>,
    pub event_type: EventType,
    pub event_data: EventData,
    pub message_data: MessageData,
    pub remote_data: RemoteData,
    pub location_data: LocationData,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub company_id: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub serial_data: Option<SerialData>,
}
```

All nested types (`RemoteData`, `LocationData`, `EventData`, `MessageData`, `SerialData`, `EventType`) retain their original names since the artifacts did not rename them.

### 4. Update Handler (`src/handler.rs`)

**Before**:

```rust
use crate::smartrak::{EventType, MessageData, RemoteData, SmarTrakEvent};

const SMARTRAK_TOPIC: &str = "realtime-r9k-to-smartrak.v1";
```

**After**:

```rust
use crate::position::{EventType, MessageData, RemoteData, PositionEvent};

const POSITION_TOPIC: &str = "realtime-r9k-to-position.v1";
```

Update all references in the `handle` function:

**Before**:

```rust
let events: Vec<SmarTrakEvent> = /* ... */;
// ...
let topic = format!("{env}-{SMARTRAK_TOPIC}");
```

**After**:

```rust
let events: Vec<PositionEvent> = /* ... */;
// ...
let topic = format!("{env}-{POSITION_TOPIC}");
```

### 5. Update Stops Module (`src/stops.rs`)

If `stops.rs` references `SmarTrakEvent` or the smartrak module:

**Before**:

```rust
use crate::smartrak::LocationData;
```

**After**:

```rust
use crate::position::LocationData;
```

The `From<StopInfo> for LocationData` impl does not change because `LocationData` was not renamed.

### 6. Update Tests (`tests/static.rs`)

**Before**:

```rust
use r9k_adapter::SmarTrakEvent;
```

**After**:

```rust
use r9k_adapter::PositionEvent;
```

Update any assertions or variable names:

```rust
// Before:
let event: SmarTrakEvent = serde_json::from_value(body).unwrap();
// After:
let event: PositionEvent = serde_json::from_value(body).unwrap();
```

### 7. Update Guest Wiring

If the guest references the type:

**Before**:

```rust
use r9k_adapter::SmarTrakEvent;
```

**After**:

```rust
use r9k_adapter::PositionEvent;
```

The messaging topic match arm may also need updating if the topic name changed:

**Before**:

```rust
t if t.contains("realtime-r9k-to-smartrak.v1") => r9k_handler(message.data()).await,
```

**After**:

```rust
t if t.contains("realtime-r9k-to-position.v1") => r9k_handler(message.data()).await,
```

### 8. Post-Structural Verification

After all structural changes, run:

```bash
cargo check
```

If compilation passes, re-inventory the crate to update file references before applying any subsequent modifying or additive changes.

### 9. CHANGELOG.md Entry

```markdown
## [Update: 2026-03-01]

### Changed
- Renamed `SmarTrakEvent` to `PositionEvent` (downstream system rebranding)
- Renamed module `smartrak` to `position`
- Updated publication topic from `realtime-r9k-to-smartrak.v1` to `realtime-r9k-to-position.v1`
```

## Structural Rename Propagation Checklist

This checklist ensures no references were missed:

- [x] Type definition renamed (`src/position.rs`)
- [x] Module file renamed (`smartrak.rs` → `position.rs`)
- [x] `src/lib.rs` module declaration updated
- [x] `src/lib.rs` `pub use` updated
- [x] `src/handler.rs` import path updated
- [x] `src/handler.rs` all type references updated
- [x] `src/handler.rs` topic constant renamed and value updated
- [x] `src/stops.rs` import path updated (if applicable)
- [x] `tests/static.rs` import updated
- [x] `tests/static.rs` all type references updated
- [x] Guest `src/lib.rs` import updated
- [x] Guest topic match arm updated
- [x] Doc comments updated to reflect new name
- [x] `cargo check` passes after all renames
- [x] Re-inventory completed

## What Was NOT Changed

- `Cargo.toml` -- crate name unchanged, no dependency changes
- `src/r9k.rs` -- input types unaffected
- `src/stops.rs` -- `StopInfo` and `LocationData` names unchanged
- `tests/provider.rs` -- MockProvider unaffected
- `.env.example` -- config keys unchanged

## Verification

- [x] Baseline `cargo test` captured
- [x] All references to `SmarTrakEvent` replaced with `PositionEvent`
- [x] All references to `smartrak` module replaced with `position`
- [x] Topic constant value updated
- [x] No remaining references to old names (grep verified)
- [x] Post-structural `cargo check` passes
- [x] No regressions: all tests pass with updated names
- [x] `cargo clippy` passes
- [x] CHANGELOG.md documents the rename
