# Output Documents

Documentation artifacts generated alongside the crate.

## Migration.md

**Create mode**: 2-3 paragraphs: key transformation decisions, areas of uncertainty, critical design choices.

**Update mode**: Update (not replace) the existing Migration.md with new entries for new TODO markers, capability gaps, and behavioral changes.

## Architecture.md

**Create mode**: 2-3 paragraphs: component behavior, data flows, error handling, provider integration.

**Update mode**: Update (not replace) the existing Architecture.md if structural changes alter the component's data flows or handler topology.

## CHANGELOG.md (update mode only)

Append entries for all changes applied in this update:

```markdown
## [Update: YYYY-MM-DD]

### Added
- New handler `POST /worksite` (CreateWorksiteRequest)
- New type `CreateWorksiteInput`

### Changed
- Added `priority` field to `WorksiteRequest`
- Updated worksite filter to include priority filtering

### Removed
- Removed `GET /legacy-status` endpoint (no longer in artifacts)
- Removed `LegacyStatusRequest` and `LegacyStatusResponse` types
```

## .env.example

```bash
# Required environment variables
ENV=dev
# One entry per Config::get key used in the crate
```

Every `Config::get` key in the code must appear in `.env.example`.
