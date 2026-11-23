# 93-improvements-review: Deferred Findings Backlog

Status: draft · Owner: pdfv · Last updated: 2026-05-15

This backlog records valid review findings that are intentionally deferred out
of the current implementation phase.

## Open M9 Findings

- **P1.** The automatic fallback loads the ten-rule `pdfv-m4` profile and
  assigns it the requested standard flavour. Switch the default validation
  path to the corresponding generated profile and migrate the affected CLI,
  library, repair, and corpus expectations. See
  [profile selection](../crates/core/src/profile.rs) and the
  [live conformance review](../docs/reviews/m9-live-conformance-review.md).
- **P1.** Official rule object types are broader after import. `CosInteger`,
  `CosReal`, `CosString`, and `CosName` share the `object` target, causing
  numeric and string expressions to run on unrelated model instances.
  Preserve source-type applicability and exercise it on the public corpus.
  See [the type mapping](../crates/core/src/profile.rs).
