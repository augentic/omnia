# TODO Markers

Rules for marking incomplete functionality in generated crates.

## Marker Format

Any functionality that cannot be fully implemented must be marked:

```rust
// TODO: <description of what the source code does>
// Reason: <why -- e.g., "No Omnia capability for EventStore.put">
// Suggested: <Omnia approach if one exists -- e.g., "Use HttpRequest::fetch to PUT raw body to REPLICATION_ENDPOINT">
```

## When to Generate TODO Markers

- **`[unknown]` artifact items** -- behavior unclear, cannot safely generate
- **`[infrastructure]` steps referencing capabilities outside the 7 Omnia traits** -- the operation has a known purpose but no direct SDK support. Generate a TODO with the original intent and suggest the nearest Omnia capability if one exists (e.g., `HttpRequest::fetch` for HTTP-based storage, `StateStore` for caching). Document the gap in Migration.md.
- **`[domain]` steps that call a named external system** (EventStore, Key Vault, third-party API, etc.) -- tag is not enough to skip the check. If the external system does not map to a provider trait bound already in scope, generate a TODO at the call site. Document the gap in Migration.md.
- **design.md Source Capabilities Summary with `[ ]` unchecked checkboxes** -- the artifact author identified the capability as potentially needed but left it unresolved. Generate a TODO at every call site that would use the corresponding provider trait; document in Migration.md with the stated purpose. Do **not** silently omit it just because the checkbox is unchecked.
- **Pre-generation checklist items** marked NO or UNCLEAR

## Capability Override for Managed Data Stores

**SKILL.md authority — overrides artifacts.** When the artifacts or source code describe access to a managed data store — Azure Table Storage, Azure Cosmos DB, Redis, or similar — but assign `HttpRequest` as the provider (e.g., design.md says "use HttpRequest for Azure Table Storage REST API"), **override the artifacts and use the correct storage trait instead**. Azure Table Storage and Cosmos DB map to `TableStore`; Redis and key-value caches map to `StateStore`. The Omnia runtime provides native adapters for these services; constructing raw HTTP requests with storage-specific authentication (SharedKey, HMAC-SHA256, SAS tokens) is always wrong. Update the handler's trait bounds accordingly. This override follows the authority hierarchy: SKILL.md > artifacts. See [anti-patterns.md](../examples/anti-patterns.md) #10 for a contrastive example.

### Recognizing Managed Data Stores in Artifacts

Look for these signals in design.md External Services, algorithm steps, or Source Capabilities Summary — even when the artifacts describe access as HTTP:

- Azure Table Storage: `@azure/data-tables`, `TableClient`, `listEntities`, `table.core.windows.net`, `fleetdata`, SharedKey auth, `AzureNamedKeyCredential`
- Azure Cosmos DB: `@azure/cosmos`, `CosmosClient`, `documents.azure.com`
- Redis: `redis`, `ioredis`, `cache.windows.net`

When recognized, replace `HttpRequest` with `TableStore` (or `StateStore` for Redis) in handler bounds and generate code using the ORM patterns from [capabilities/tablestore.md](../examples/capabilities/tablestore.md).

### TableStore / StateStore Inference

When the derived capabilities include TableStore or StateStore and the design.md Business Logic (or optional "Data access" subsection per block) gives **actionable cues** — e.g. "Table access: SELECT entity WHERE col=$1", "Cache: get/set/delete key pattern" — generate real code using [capabilities/tablestore.md](../examples/capabilities/tablestore.md) and [capabilities/statestore.md](../examples/capabilities/statestore.md). Infer table/entity and key pattern from step text and design.md External Services. **Prefer ORM** (SelectBuilder, InsertBuilder, UpdateBuilder, Filter) for CRUD and simple queries; use **raw SQL** (TableStore::query / TableStore::exec) only when the artifacts or legacy indicate GeoSearch/spatial (e.g. PostGIS), nested subqueries, or complex transactional flows. Only generate a TODO when the step is too vague (e.g. "cache invalidation" with no key pattern). Do not require full SQL or full key lists in the artifacts; canonical one-line hints are enough.

## Startup Cache → On-Demand Cache-Aside

**Never assume external cron/ETL.** When the artifacts describe a legacy pattern of "load data from a data store on startup into an in-memory cache" (with optional periodic refresh via `setTimeout`/`setInterval`), the WASM translation is **on-demand cache-aside** within the handler:

1. Check `StateStore` for cached data.
2. On cache miss, query the original data source using the appropriate trait (`TableStore` for databases/managed table stores, `HttpRequest` for external APIs).
3. Write the fetched data to `StateStore` with a TTL (replacing periodic refresh with TTL-based expiry).
4. Return the data.

The handler must include **both** `StateStore` and the data source trait (e.g., `Config + TableStore + StateStore`) in its provider bounds. Do **not** assume a separate cron/ETL component pre-populates the cache — the handler is self-sufficient and fetches data on demand. Do **not** drop the data-fetching logic from the handler or document it as "handled by external component" in Migration.md. See [capabilities/statestore.md](../examples/capabilities/statestore.md) for the cache-aside pattern and the Capability Selection Summary's "Cache-aside (TableStore + StateStore)" note.

## Critical Rules

**Never silently drop artifact steps.** Every `[domain]`, `[infrastructure]`, and `[mechanical]` step in the artifacts must produce either generated code or a TODO marker.

**Migration.md is not a substitute for TODO markers.** If a behavior is documented only in Migration.md and has no TODO in the code, the engineer reading the code cannot see it. Both are required: a TODO at the call site AND a note in Migration.md.

**"No library/SDK equivalent" is not a valid reason to drop behavior.** If a TypeScript dependency (e.g. `at-connector-common`'s EventStore) has no Rust port, that is irrelevant — the _intent_ of the call (e.g. replicate raw body to an endpoint when `REPLICATION_ENDPOINT` is set) maps to Omnia capabilities (`HttpRequest::fetch` or a second `Publisher::send`). Generate a TODO with the business intent, not the TypeScript library name.
