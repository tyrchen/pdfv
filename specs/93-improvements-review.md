# 93-improvements-review: Deferred Findings Backlog

Status: draft · Owner: pdfv · Last updated: 2026-05-15

This backlog records valid review findings that are intentionally deferred out
of the current implementation phase.

## Open M9 Findings

- **P1.** Context-dependent targets such as `CosLang` and `CosTextString`
  still map to the generic `object` target. Preserve their
  actual use-site semantics and compare each assertion on passing and failing
  fixtures. See [the type mapping](../crates/core/src/profile.rs).
- **P1.** PDF/A-3 rule `6.8-4` needs associated-file evidence from the
  applicable `/AF` arrays. Its current generic property treats
  `AFRelationship` and name-tree membership as association evidence.
  Verify reachable catalog, page, structure, resource, and marked-content
  association sites against the oracle. See
  [the associated-file property](../crates/core/src/validation.rs).

## Resolved M9 Findings

- **P1.** Automatic fallback selects the generated conformance profile for the
  configured flavour. The public oracle covers absent XMP and unsupported XMP
  identification. See the [live conformance review](../docs/reviews/m9-live-conformance-review.md).
- **P1.** PDF/UA and WTPDF file specification rules use the physical dictionary
  and `EmbeddedFiles` name-tree scopes. The four targeted public PDF/UA fixtures
  agree on assertion outcomes and counts. See the
  [file specification follow-up](../docs/reviews/m9-live-conformance-review.md#pdfua-file-specifications).
