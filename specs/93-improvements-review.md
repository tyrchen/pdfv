# 93-improvements-review: Deferred Findings Backlog

Status: draft · Owner: pdfv · Last updated: 2026-11-24

This backlog records valid review findings that are intentionally deferred out
of the current implementation phase.

## Open M9 Findings

- **P2.** PDF/UA-1 `7.21.4.1-2` still needs glyph presence evidence
  from the selected glyph rather than the font dictionary. Three negative
  controls also need conclusive Unicode fallback evidence for
  `7.21.7-1` and `7.21.7-2` in an otherwise readable TrueType program.
  [Glyph comparison and remaining differences](../docs/reviews/m9-live-conformance-review.md#built-in-truetype-encoding-and-glyph-widths)
- **P2.** Audit PDF/UA-1 annotation rule applicability in the three
  `scope-appearance`, `scope-also-appearance`, and `scope-unused-form`
  controls. Two report unavailable annotation properties. The third
  fails both `7.18.6.2` checks, which Greenfield leaves untested.
  [Annotation traversal](../crates/core/src/validation.rs)

## Resolved M9 Findings

- **P2.** PDF/UA-1 width evidence combines equivalent glyphs across page
  and Form use sites. Rendering mode, marked content, structure context,
  and annotation appearance preserve distinct observations. A confirmed
  corrupt TrueType program fails its embedding check and has no program
  width. [Glyph comparison](../docs/reviews/m9-live-conformance-review.md#built-in-truetype-encoding-and-glyph-widths)
- **P2.** PDF/A-1 uses the built-in encoding of each nonembedded
  standard Type 1 font and applies the mode 3 exemption to actual text
  use. Both invisible Helvetica controls validate.
  [Font use and standard encodings](../docs/reviews/m9-live-conformance-review.md#font-use-and-standard-type-1-encodings)
- **P2.** XPath values use bounded syntax and core function signature checks.
  PDF/A identification uses the original RDF conformance text and detects
  a present empty value. The auto-selection claim remains normalized.
  [XPath and identification review](../docs/reviews/m9-live-conformance-review.md#xpath-functions-and-pdfa-identification)
- **P2.** Standard security handler revisions 2–4 append the prefix of
  the password padding sequence. Independent libqpdf fixtures verify both
  user and owner authentication. Decrypted PDF/A inputs still fail the
  rule prohibiting the trailer's `/Encrypt` declaration.
  [Legacy password review](../docs/reviews/m9-live-conformance-review.md#legacy-pdf-passwords-and-minimal-builds)
- **P2.** Minimal builds retain the SHA-2 dependency used by language
  validation. The core test and lint gates cover disabled default features.
  [Core feature flags](./61-crates-and-features.md#4-feature-flags)
- **P1.** Embedded PDF conformance uses the MIME rule's `/EF` stream
  selection and the attached document's own PDF/A profile. Nested
  validation shares resource limits and preserves undecidable results.
  [Embedded PDF review](../docs/reviews/m9-live-conformance-review.md#embedded-pdf-conformance)
- **P1.** Linked xref entries select the active generation and byte offset or
  object-stream member. The object store honours deletions, hybrid precedence,
  and linearized section order. See
  [the xref review](../docs/reviews/m9-live-conformance-review.md#active-cross-reference-entries).
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
- **P1.** Signature rules use AcroForm signature field values, typed
  signature references, physical Contents spans, and DER SignedData.
  The remaining false-invalid decision in the 15-profile audit is resolved.
  See [the permission and signature audit](../docs/reviews/m9-live-conformance-review.md#digital-signatures-and-document-permissions).
- **P1.** UTF-8 names are checked at their font, colourant, and structure
  use sites, and private-use glyphs check their scoped replacement text.
  The twelve upstream fixtures agree on all 22 targeted rule counts.
  See [the name and glyph audit](../docs/reviews/m9-live-conformance-review.md#utf-8-names-and-glyph-replacement-text).
- **P1.** MacRoman glyph metrics, Type 1 font subset coverage, and
  glyph-level Unicode evidence resolve the four incomplete decisions in
  the 15-profile audit. All 30 outcomes now agree with Greenfield.
  See [the font audit](../docs/reviews/m9-live-conformance-review.md#type-1-font-subsets-and-macintosh-character-maps).
