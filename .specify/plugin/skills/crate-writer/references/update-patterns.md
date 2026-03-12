# Update Patterns

Concrete patterns for each of the four update categories. Each pattern shows the before state, the artifact change, and the after state with the specific edits required.

## Structural Patterns

Structural changes affect naming, relationships, or organization. They propagate across multiple files and must be applied first to establish a stable foundation for subsequent changes.

### Rename a Type

**Artifact Change**: Type `OrderEvent` renamed to `PurchaseEvent` in Domain Model section.

**Files affected**: Every file that references the type.

**Before** (`src/types.rs`):

```rust
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OrderEvent {
    pub order_id: String,
    pub amount: f64,
    pub received_at: DateTime<Utc>,
}
```

**After** (`src/types.rs`):

```rust
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PurchaseEvent {
    pub order_id: String,
    pub amount: f64,
    pub received_at: DateTime<Utc>,
}
```

**Propagation checklist**:

- [ ] Struct definition renamed
- [ ] All `use` imports updated
- [ ] All variable bindings and function parameters updated
- [ ] Handler `type Output` updated if applicable
- [ ] `IntoBody` impl updated
- [ ] `From` impls updated
- [ ] Test fixtures and assertions updated
- [ ] Doc comments updated
- [ ] Guest wiring imports updated

### Rename a Module

**Artifact Change**: Handler moved from single-handler layout to multi-handler barrel.

**Before** (single handler):

```
src/
├── lib.rs      # mod handler; pub use handler::*;
└── handler.rs  # single Handler<P> impl
```

**After** (multi-handler barrel):

```
src/
├── lib.rs          # mod handlers; pub use handlers::*;
├── handlers.rs     # mod existing; mod new_handler; pub use ...;
└── handlers/
    ├── existing.rs # moved from handler.rs
    └── new_handler.rs
```

**Steps**:

1. Create `src/handlers.rs` barrel module
2. Create `src/handlers/` directory
3. Move `handler.rs` content to `src/handlers/existing.rs`
4. Update `src/lib.rs`: replace `mod handler` with `mod handlers`
5. Update `pub use` declarations
6. Run `cargo check` to verify

### Split a Handler

**Artifact Change**: A handler that processed multiple message types is split into separate handlers.

**Before** (`src/handler.rs`):

```rust
async fn handle<P>(_owner: &str, request: MultiMessage, provider: &P) -> Result<Reply<()>>
where
    P: Config + Publish,
{
    match request.message_type.as_str() {
        "type_a" => handle_type_a(&request, provider).await,
        "type_b" => handle_type_b(&request, provider).await,
        _ => Err(bad_request!("unknown message type")),
    }
}
```

**After** (two separate handler files):

`src/handlers/type_a.rs`:

```rust
async fn handle<P>(_owner: &str, request: TypeAMessage, provider: &P) -> Result<Reply<()>>
where
    P: Config + Publish,
{
    // type_a logic extracted here
    Ok(Reply::ok(()))
}
```

`src/handlers/type_b.rs`:

```rust
async fn handle<P>(_owner: &str, request: TypeBMessage, provider: &P) -> Result<Reply<()>>
where
    P: Config + Publish,
{
    // type_b logic extracted here
    Ok(Reply::ok(()))
}
```

**Guest wiring update**: Replace single topic arm with two separate topic arms.

## Subtractive Patterns

Subtractive changes remove code. Every removal is documented in CHANGELOG.md.

### Remove an HTTP Endpoint

**Artifact Change**: `GET /legacy-status` endpoint no longer appears in API Contracts.

**Steps**:

1. **Delete handler**: Remove `src/handlers/legacy_status.rs` (or remove the handler function from a shared file)
2. **Update barrel**: Remove `mod legacy_status;` and `pub use legacy_status::*;` from `src/handlers.rs`
3. **Delete types**: Remove `LegacyStatusRequest` and `LegacyStatusResponse` from `src/types.rs` (only if no other handler uses them)
4. **Delete tests**: Remove `tests/legacy_status.rs`
5. **Update guest**: Remove the route from `$PROJECT_DIR/src/lib.rs`:

   ```rust
   // REMOVE this line:
   .route("/legacy-status", get(legacy_status_handler))
   ```

6. **Remove import**: Remove `use my_crate::LegacyStatusRequest;` from guest
7. **Remove handler function**: Remove `async fn legacy_status_handler(...)` from guest
8. **Clean dependencies**: Remove unused dependencies from `Cargo.toml`
9. **Document**: Add entry to CHANGELOG.md under `### Removed`

### Remove a Messaging Topic Handler

**Artifact Change**: Topic `events.legacy.v1` no longer appears in API Contracts.

**Steps**:

1. Delete or remove the handler implementation
2. Remove the topic match arm from the guest messaging dispatcher:

   ```rust
   // REMOVE this arm:
   t if t.contains("events.legacy.v1") => legacy_handler(message.data()).await,
   ```

3. Remove the handler function from the guest
4. Remove the import from the guest
5. Delete corresponding tests
6. Document in CHANGELOG.md

### Remove a Type

**Before removing a type**, verify it is not referenced by any remaining handler:

```text
Does any remaining handler reference this type?
├─ YES → Do NOT remove; the type is still needed
└─ NO → Safe to remove
    └─ Is the type part of a public API response?
       ├─ YES → Mark as BREAKING in CHANGELOG.md
       └─ NO → Remove and document
```

## Modifying Patterns

Modifying changes update existing code without adding or removing top-level items.

### Add a Field to an Existing Type

**Artifact Change**: New field `priority` (optional, string) added to `WorksiteRequest` in Domain Model.

**Before** (`src/handlers/worksite.rs`):

```rust
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct WorksiteRequest {
    pub worksite_code: String,
    #[serde(rename = "expanded")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub include_tmps: Option<bool>,
}
```

**After** (`src/handlers/worksite.rs`):

```rust
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct WorksiteRequest {
    pub worksite_code: String,
    #[serde(rename = "expanded")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub include_tmps: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority: Option<String>,
}
```

**Propagation**:

- Update any filter builders or query construction that should use the new field
- Update test fixtures to include the new field in relevant scenarios
- Update MockProvider if the field triggers new provider calls

### Change Business Logic

**Artifact Change**: Validation threshold `MAX_DELAY_SECS` changed from 60 to 120 in Business Logic block.

**Before** (`src/r9k.rs`):

```rust
const MAX_DELAY_SECS: i64 = 60;
```

**After** (`src/r9k.rs`):

```rust
const MAX_DELAY_SECS: i64 = 120;
```

**Test update**: Adjust test assertions that depend on the threshold boundary.

### Add a Provider Trait Bound

**Artifact Change**: Handler now requires `StateStore` for caching (new provider in Required Providers).

**Before** (`src/handler.rs`):

```rust
async fn handle<P>(_owner: &str, request: MyRequest, provider: &P) -> Result<Reply<()>>
where
    P: Config + HttpRequest,
```

**After** (`src/handler.rs`):

```rust
async fn handle<P>(_owner: &str, request: MyRequest, provider: &P) -> Result<Reply<()>>
where
    P: Config + HttpRequest + StateStore,
```

**Propagation**:

- Update `Handler<P>` impl bounds to match
- Add `use omnia_sdk::StateStore;` import
- Update MockProvider:

  ```rust
  impl StateStore for MockProvider {
      async fn get(&self, key: &str) -> Result<Option<Vec<u8>>> {
          Ok(None) // or return test fixture data
      }
      async fn set(&self, key: &str, value: &[u8]) -> Result<()> {
          Ok(())
      }
      // ... implement remaining StateStore methods
  }
  ```

- Update guest Provider if not already implementing `StateStore`:

  ```rust
  impl StateStore for Provider {}
  ```

### Change Error Handling

**Artifact Change**: A condition that previously returned `BadRequest` now returns `NotFound`.

**Before** (`src/handler.rs`):

```rust
let item = items.first().ok_or_else(|| {
    bad_request!("no items found")
})?;
```

**After** (`src/handler.rs`):

```rust
let item = items.first().ok_or_else(|| Error::NotFound {
    code: "item_not_found".to_string(),
    description: "no items found".to_string(),
})?;
```

**Test update**: Change test assertions from expecting status 400 to status 404.

### Change Serde Attributes

**Artifact Change**: An input-only type now needs to be round-tripped (cached in StateStore).

**Before** (`src/types.rs`):

```rust
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct InboundMessage {
    #[serde(rename(deserialize = "sourceField"))]
    pub source_field: Option<String>,
}
```

**After** (`src/types.rs`):

```rust
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct InboundMessage {
    #[serde(rename = "sourceField")]
    pub source_field: Option<String>,
}
```

Note: `rename(deserialize = ...)` changes to `rename = ...` for round-trip types, and `Serialize` is added to derives.

## Additive Patterns

Additive changes add new code following crate-writer patterns exactly. The existing crate structure determines where new code goes.

### Add a Handler to a Single-Handler Crate

When a single-handler crate gains a second handler, it transitions to multi-handler layout.

**Before** (single handler):

```
src/
├── lib.rs
├── handler.rs
└── types.rs
```

**After** (multi-handler):

```
src/
├── lib.rs          # updated: mod handlers; (replaces mod handler;)
├── handlers.rs     # barrel: mod existing; mod new_handler; pub use ...;
├── handlers/
│   ├── existing.rs # content from former handler.rs
│   └── new_one.rs  # new handler
└── types.rs
```

This is a combined structural + additive change. The structural transition happens first, then the additive handler is placed into the new layout.

### Add a Handler to a Multi-Handler Crate

**Steps**:

1. Create `src/handlers/new_handler.rs` following the Handler pattern
2. Add `mod new_handler;` and `pub use new_handler::*;` to `src/handlers.rs`
3. Add new types to `src/types.rs` or create domain-specific type modules
4. Add dependencies to `Cargo.toml` if needed
5. Create `tests/new_handler.rs` with happy path and error tests
6. Update MockProvider if new traits are needed
7. Add guest wiring (route/topic/import)

### Add a New Type

**Steps**:

1. Add the type definition to the appropriate module (`src/types.rs` or a domain module)
2. Follow the serde conventions:
   - Input-only: `Deserialize`, `#[serde(default)]`, `#[serde(rename(deserialize = "..."))]`
   - Output: `Serialize + Deserialize`, `#[serde(rename_all = "camelCase")]`
   - Round-trip: `Serialize + Deserialize`, `#[serde(rename = "...")]`
3. Add doc comments
4. If it's an HTTP response type, implement `IntoBody`
5. Update tests to use the new type

### Add a Test

**Steps**:

1. Create the test file in `tests/`
2. Import MockProvider from `tests/provider.rs`
3. Follow the test structure:

   ```rust
   #[tokio::test]
   async fn happy_path() {
       let provider = MockProvider::new();
       let client = Client::new("owner").provider(provider.clone());
       let request = MyRequest { /* ... */ };
       let response = client.request(request).await.expect("should succeed");
       assert_eq!(response.status, 200);
   }
   ```

4. Add error case tests for validation failures
5. Update MockProvider if new fixtures or trait methods are needed
