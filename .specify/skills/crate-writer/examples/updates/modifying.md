# Modifying Example: Change Business Logic in r9k-adapter

Modifying the r9k-adapter crate to change validation thresholds and add a new optional field to the output event type. This demonstrates changes to constants, type definitions, and test assertions.

## Starting State

The `r9k-adapter` single-handler crate as shown in the [crate-writer single-handler example](../single-handler.md):

```
crates/r9k-adapter/
├── Cargo.toml
├── src/
│   ├── lib.rs
│   ├── handler.rs
│   ├── r9k.rs
│   ├── smartrak.rs
│   └── stops.rs
├── tests/
│   ├── provider.rs
│   └── static.rs
└── data/static/
```

## Artifact Changes

The updated artifacts contain two changes in the Business Logic section:

### Change 1: Validation threshold

```markdown
#### Business Logic Block: Validate Train Update

- Pre-conditions:
  - changes array is non-empty
  - first change has arrival or departure time > 0
- Algorithm:
  1. [domain] Extract seconds since midnight from first change
  2. [domain] Rebuild event timestamp from creation date + seconds offset
  3. [domain] Compare event timestamp to current time
  4. [domain] Reject if delay exceeds **120 seconds** (was 60)
  5. [domain] Reject if event is more than **45 seconds** in the future (was 30)
```

### Change 2: New output field

```markdown
#### Domain Model > SmarTrakEvent

- `received_at`: DateTime<Utc>
- `event_type`: EventType
- `event_data`: EventData
- `message_data`: MessageData
- `remote_data`: RemoteData
- `location_data`: LocationData
- `company_id?`: u64
- `serial_data?`: SerialData
- `source_system?`: string          ← NEW FIELD
```

## Derived Change Set

- **Category**: Modifying (both changes are within existing definitions)
- **Changes**:
  1. `src/r9k.rs`: Update `MAX_DELAY_SECS` from 60 to 120
  2. `src/r9k.rs`: Update `MIN_DELAY_SECS` from -30 to -45
  3. `src/smartrak.rs`: Add `source_system: Option<String>` to `SmarTrakEvent`
  4. `tests/static.rs`: Update test assertions for new thresholds

## Applied Changes

### 1. Update Validation Constants (`src/r9k.rs`)

**Before**:

```rust
const MAX_DELAY_SECS: i64 = 60;
const MIN_DELAY_SECS: i64 = -30;
```

**After**:

```rust
const MAX_DELAY_SECS: i64 = 120;
const MIN_DELAY_SECS: i64 = -45;
```

No other changes to `r9k.rs`. The validation logic in `TrainUpdate::validate()` already uses these constants, so the behavior change propagates automatically.

### 2. Add Field to Output Type (`src/smartrak.rs`)

**Before**:

```rust
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

**After**:

```rust
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
    /// Source system identifier (e.g., "r9k", "gtfs").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_system: Option<String>,
}
```

### 3. Update Event Construction (`src/handler.rs` or `src/r9k.rs`)

Wherever `SmarTrakEvent` is constructed, the new field must be included. Since `SmarTrakEvent` derives `Default` and the field is `Option<String>`, existing construction using struct update syntax (`..Self::default()`) automatically handles this. If construction is explicit, add:

```rust
SmarTrakEvent {
    received_at: Utc::now(),
    event_type: EventType::Location,
    // ... existing fields ...
    source_system: Some("r9k".to_string()),
}
```

### 4. Update Tests (`tests/static.rs`)

Update threshold-boundary tests:

**Before** (testing old 60-second threshold):

```rust
#[tokio::test]
async fn rejects_outdated_message() {
    // Set up a message that is 65 seconds old
    let delay = 65;
    // ...
    let result = client.request(request).await;
    assert!(result.is_err());
}
```

**After** (testing new 120-second threshold):

```rust
#[tokio::test]
async fn rejects_outdated_message() {
    // Set up a message that is 125 seconds old (exceeds new 120s threshold)
    let delay = 125;
    // ...
    let result = client.request(request).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn accepts_message_within_new_threshold() {
    // Set up a message that is 90 seconds old (within new 120s threshold)
    let delay = 90;
    // ...
    let result = client.request(request).await;
    assert!(result.is_ok());
}
```

Update future-event tests similarly for the new -45 second boundary.

If test assertions check the serialized output, verify `source_system` appears:

```rust
let body: serde_json::Value = serde_json::from_slice(&response.body).unwrap();
assert_eq!(body["sourceSystem"], "r9k");
```

## What Was NOT Changed

The following files were not modified because the changes do not affect them:

- `Cargo.toml` -- no new dependencies
- `src/lib.rs` -- no new modules or error variants
- `src/stops.rs` -- unrelated domain helper
- `tests/provider.rs` -- MockProvider already implements needed traits
- Guest wiring -- no route/topic changes

This demonstrates the "preserve unchanged code" hard rule.

## CHANGELOG.md Entry

```markdown
## [Update: 2026-03-01]

### Changed
- Increased `MAX_DELAY_SECS` validation threshold from 60 to 120 seconds
- Increased future-event rejection threshold from 30 to 45 seconds
- Added optional `source_system` field to `SmarTrakEvent` output type
```

## Verification

- [x] Baseline `cargo test` captured (all existing tests pass)
- [x] Only files in the change set were modified (4 files)
- [x] Validation constants match artifact specification
- [x] New field follows output type conventions (`skip_serializing_if`, doc comment)
- [x] Test assertions updated for new threshold boundaries
- [x] No regressions: all previously-passing tests still pass
- [x] `cargo check` passes
- [x] `cargo clippy` passes
- [x] CHANGELOG.md updated
