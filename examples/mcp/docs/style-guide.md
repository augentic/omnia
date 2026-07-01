# Widget Service Style Guide

These conventions keep widget code consistent and reviewable.

## Naming

- Widget labels are lower-case kebab-case: `left-flange`, not `Left Flange`.
- Identifiers are ULIDs, never sequential integers.

## Errors

- Return `4xx` for caller mistakes and `5xx` only for genuine service faults.
- Every error body carries a `code` and a human-readable `description`.

## Testing

- Every state transition has a test proving the illegal reverse transition is
  rejected. A widget that can move backwards is a bug.
