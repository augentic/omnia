# Change Classification

Rules for classifying differences between new artifacts and an existing crate into the four update categories: structural, subtractive, modifying, and additive.

## Classification Decision Tree

For each difference found when comparing the new artifacts against the crate inventory:

```text
Is the item in the new artifacts but NOT in the inventory?
├─ YES → ADDITIVE (new handler, type, endpoint, config key)
└─ NO
   Is the item in the inventory but NOT in the new artifacts?
   ├─ YES → SUBTRACTIVE (removed handler, type, endpoint, config key)
   └─ NO (item exists in both)
      Has the item's name, location, or structural role changed?
      ├─ YES → STRUCTURAL (rename, split, merge, reorganize)
      └─ NO
         Has the item's content (fields, logic, bounds, attributes) changed?
         ├─ YES → MODIFYING (updated logic, new field, changed bounds)
         └─ NO → NO CHANGE (skip)
```

## Detailed Classification Rules

### Additive

An item is **additive** when:

| Condition | Example |
| --- | --- |
| Handler/endpoint in artifacts, not in crate | New `POST /orders` endpoint in API Contracts |
| Type in artifacts Domain Model, not in crate | New `OrderPriority` enum |
| Config key in artifacts Configuration, not in `.env.example` | New `PAYMENT_API_URL` key |
| Provider trait in artifacts Required Providers, not bound in any handler | `TableStore` first used |
| Business logic block with no corresponding code | New validation rule |
| BDD scenario with no corresponding test | New acceptance criterion |

### Subtractive

An item is **subtractive** when:

| Condition | Example |
| --- | --- |
| Handler in crate, not in artifacts API Contracts | `GET /legacy-status` no longer specified |
| Type in crate, not in artifacts Domain Model | `LegacyStatusResponse` no longer referenced |
| Config key in `.env.example`, not in artifacts Configuration | `LEGACY_API_URL` no longer needed |
| Test covering removed behavior | `tests/legacy_status.rs` tests removed endpoint |

**Safety check before removing**:

- Verify the item is truly absent from the artifacts (search all sections, not just the obvious one)
- Verify no remaining handler depends on the type/config key being removed
- If `$CHANGE_DESCRIPTION` is provided and does not mention this removal, flag it for confirmation

### Modifying

An item is **modifying** when it exists in both the artifacts and the crate, but its content differs:

| Condition | Example |
| --- | --- |
| Type has different fields | Artifacts add `priority: Option<String>` to `WorksiteRequest` |
| Type has different serde attributes | Input-only type now needs `Serialize` for caching |
| Handler has different business logic | Validation threshold changed, algorithm updated |
| Handler has different provider bounds | New `StateStore` bound added |
| Handler has different input/output types | Input type changed from `Vec<u8>` to `String` |
| Error handling changed | `BadRequest` replaced with `NotFound` for a condition |
| Constant value changed | `MAX_DELAY_SECS` from 60 to 120 |

### Structural

An item is **structural** when its identity, location, or organizational role has changed:

| Condition | Example |
| --- | --- |
| Type renamed | `OrderEvent` → `PurchaseEvent` in Domain Model |
| Module renamed | `order.rs` → `purchase.rs` |
| Handler split | Single handler processing multiple types → separate handlers |
| Handler merged | Two handlers with similar logic → one handler |
| Crate layout change | Single-handler → multi-handler barrel |
| Domain model relationships changed | One-to-many becomes many-to-many |
| Enum variant renamed | `Active` → `InProgress` |

## Edge Cases

### Modifying vs Structural

When a change could be classified as either modifying or structural, apply this rule:

```text
Does the change require updating references in OTHER files?
├─ YES (type name changed, module moved) → STRUCTURAL
└─ NO (change is contained within the item's own definition)
   ├─ Adding/removing a field → MODIFYING
   ├─ Changing a field's type → MODIFYING
   ├─ Changing serde attributes → MODIFYING
   └─ Changing function body → MODIFYING
```

**Concrete examples**:

| Change | Classification | Reason |
| --- | --- | --- |
| Add `priority` field to `WorksiteRequest` | Modifying | Change is within the struct definition |
| Rename `WorksiteRequest` to `SiteRequest` | Structural | Every file referencing the type must update |
| Change `pub worksite_code: String` to `pub site_code: String` | Modifying | Field rename within the struct; serde attribute handles wire format |
| Split `types.rs` into `input_types.rs` + `output_types.rs` | Structural | Module declarations and imports change |
| Add `Serialize` derive to an existing type | Modifying | Change is within the type definition |

### Additive vs Structural

When adding a handler to a single-handler crate:

```text
Does the crate currently have a single handler (src/handler.rs)?
├─ YES → This is STRUCTURAL + ADDITIVE
│  First: structural transition to multi-handler layout
│  Then: additive placement of new handler
└─ NO (already multi-handler) → ADDITIVE only
```

### Subtractive: Shared Types

When removing a handler, its types may or may not need removal:

```text
Is the type used ONLY by the handler being removed?
├─ YES → SUBTRACTIVE (remove with the handler)
└─ NO (shared with other handlers)
   ├─ Type unchanged → No action needed
   └─ Type needs modification → Separate MODIFYING change
```

### Rename Detection

Distinguishing a rename (structural) from a remove + add (subtractive + additive):

```text
Do the new artifacts contain a type/handler with:
  - Same or similar fields/logic as the removed item?
  - Different name?
├─ YES → Likely a RENAME (structural)
│  Verify by comparing:
│  - Field names and types (>80% match → rename)
│  - Business logic steps (same algorithm → rename)
│  - Provider bounds (same bounds → rename)
└─ NO → Separate SUBTRACTIVE + ADDITIVE
```

If `$CHANGE_DESCRIPTION` mentions a rename, trust it over heuristic matching.

## Too Complex for Automation

Flag a change as **too complex** and recommend greenfield regeneration when:

| Condition | Recommendation |
| --- | --- |
| >50% of handler logic rewritten | Re-run `crate-writer` in create mode for this handler |
| Domain model fundamentally restructured (>3 type renames + relationship changes) | Re-run full pipeline |
| Handler input type changes from one category to another (e.g., messaging → HTTP) | Re-run full pipeline |
| Crate purpose changed (e.g., adapter → API service) | Re-run full pipeline |

When aborting, document the rationale in Migration.md and recommend the appropriate pipeline invocation.

## Change Description Interaction

When `$CHANGE_DESCRIPTION` is provided:

1. **Validate alignment**: The derived change set should be consistent with the description. If the description says "add priority filtering" but the change set includes removing an endpoint, flag the discrepancy.

2. **Resolve ambiguity**: If a difference could be classified multiple ways, use the description to choose. Example: description says "rename OrderEvent to PurchaseEvent" → classify as structural, not remove + add.

3. **Scope focusing**: If the artifacts have many differences but the description mentions only specific changes, prioritize the described changes. Apply undescribed changes only if they are clearly required by the artifacts (e.g., the artifacts no longer have a section for a removed endpoint).

4. **No override**: The description cannot override the artifacts. If the description says "add field X" but the artifacts do not contain field X, do not add it. The artifacts are authoritative (authority hierarchy level 2).
