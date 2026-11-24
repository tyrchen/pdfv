# 93-improvements-review: Deferred Findings Backlog

Status: draft · Owner: pdfv · Last updated: 2026-05-15

This backlog records valid review findings that are intentionally deferred out
of the current implementation phase.

## Open M9 Findings

- **P2.** Two controls that reuse a font across a page and Form report an
  extra passing PDF/UA-1 `7.21.5-1` assertion. Audit glyph identity across
  page, Form, structure-parent, and appearance views before combining
  equivalent observations. An unreadable font program also produces
  unavailable width evidence and needs a conformance decision backed by
  the embedded-font rules. See
  [the glyph measurement](../crates/core/src/validation/glyphs.rs) and
  [the live comparisons](../docs/reviews/m9-live-conformance-review.md#built-in-truetype-encoding-and-glyph-widths).
- **P2.** Resolve XPath 1.0 core function names and arity at validation time
  within the existing syntax budget. Feed the exact RDF value to PDF/A
  `conformance` checks alongside the normalized claim used for detection.
  See [the XMP audit](../docs/reviews/m9-live-conformance-review.md#qualified-xmp-property-validation).
- **P1.** The 15-profile audit found six incomplete decisions and
  one false-invalid decision. Scope the transparency rules to actual groups
  and their use. Resolve the
  remaining object-stream, CIDSet, glyph-width, and Unicode evidence. The
  [live audit](../docs/reviews/m9-live-conformance-review.md#xmp-extension-resources-and-annotation-appearances)
  records the rule IDs and representative fixtures.
- **P1.** Embedded PDF conformance checks `isValidPDFA12` and
  `isValidPDFA124` still need evidence from `/EF` stream traversal and
  bounded nested validation. Reuse the MIME rule's
  [attachment selection](../crates/core/src/validation/file_spec.rs).
- **P1.** Resolve active object identity across incremental xref sections for
  generation changes, free entries, and compressed replacements. Linked
  revision detection identifies the current sections. The object store still
  needs authoritative entry selection beyond sequential replacements. See
  [the parser design](./11-parser-core-design.md) and
  [the remaining audit](./29-rule-check-completion-impl-plan.md).

## Resolved M9 Findings

- **P1.** Automatic fallback selects the generated conformance profile for the
  configured flavour. The public oracle covers absent XMP and unsupported XMP
  identification. See the [live conformance review](../docs/reviews/m9-live-conformance-review.md).
- **P1.** PDF/UA and WTPDF file specification rules use the physical dictionary
  and `EmbeddedFiles` name-tree scopes. The four targeted public PDF/UA fixtures
  agree on assertion outcomes and counts. See the
  [file specification follow-up](../docs/reviews/m9-live-conformance-review.md#pdfua-file-specifications).
- **P1.** The language rule uses scoped PDF text evidence. Seventy-two public
  controls agree with the oracle on language assertions. See
  [the language review](../docs/reviews/m9-live-conformance-review.md#language-and-incremental-revisions).
- **P1.** `CosActualText`, `CosAlt`, and `CosTextString` use typed
  evidence from PDF text use sites. All 170 targeted rule comparisons agree
  with veraPDF. The PDF/UA-2 supplemental document decisions now match.
  See [the text-string review](../docs/reviews/m9-live-conformance-review.md#private-use-code-points-in-text-strings).
- **P1.** PDF/UA-1 glyph widths and TrueType `cmap` rules use embedded
  font-program evidence on both supplemental fixtures. Their 6 and 15
  passing width assertions and their document decisions match veraPDF.
  See [the font review](../docs/reviews/m9-live-conformance-review.md#built-in-truetype-encoding-and-glyph-widths).
- **P1.** PDF/A-3 attachment rules use real `/AF` references and
  `/EF`-selected streams. Every clause 6.8 assertion agrees with veraPDF
  across 136 controls. See
  [the attachment review](../docs/reviews/m9-live-conformance-review.md#pdfa-3-associated-files-and-profile-audit).
- **P1.** Image bit-depth and DeviceN rules use the objects reached through
  actual image and colour-space use. The 12 upstream PDF/A-3b attachment
  fixtures yield zero incomplete decisions. See
  [the scope review](../docs/reviews/m9-live-conformance-review.md#image-component-depth-and-devicen-scope).
- **P1.** RDF wrapper syntax stays outside the XMP property inventory,
  extension rules check immediate declaration fields, and annotation
  rules use the resolved appearance dictionary and field type. All 12
  upstream PDF/A-3b attachment documents match veraPDF. See
  [the XMP and appearance review](../docs/reviews/m9-live-conformance-review.md#xmp-extension-resources-and-annotation-appearances).
- **P1.** Scoped XMP property rules evaluate qualified RDF names and values.
  The 575-fixture audit now has zero false-compliant or false-invalid
  document decisions. See
  [the final property comparison](../docs/reviews/m9-live-conformance-review.md#qualified-xmp-property-validation).
- **P2.** Object-level XMP streams now produce scoped packet, property,
  and extension-schema assertions. The three affected PDF/A-2b fixtures
  agree on all 63 schema rule comparisons. See
  [the object-level audit](../docs/reviews/m9-live-conformance-review.md#object-level-xmp-packets).
