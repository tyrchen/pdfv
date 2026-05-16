# 93-improvements-review: Deferred Findings Backlog

Status: draft · Owner: pdfv · Last updated: 2026-05-15

This backlog records valid review findings that are intentionally deferred out
of the current implementation phase.

## Phase 5

| Severity | Citation | Finding | Fix shape |
| --- | --- | --- | --- |
| P2 | `crates/core/src/validation.rs:238` | M1 page/font/annotation/output-intent/content-stream wrappers are precomputed before traversal rather than fully materialized from each wrapper's `linked_objects` call. Active `ResourceLimits` now bound the traversal, so this does not block M1, but it still diverges from the ideal lazy graph in `13-validation-engine-design.md`. | Refactor `ModelObjectRef`/`ModelLinks` so child wrappers can be created on demand from `ValidationSession` storage while preserving stable identities and lifetimes. |
