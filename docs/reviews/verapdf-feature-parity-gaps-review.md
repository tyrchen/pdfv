# Review: veraPDF Feature Parity Gaps And Improvement Plan

Status: Done · Owner: pdfv · Date: 2026-05-17

Vendor pins:
`vendors/veraPDF-parser` @ `5164caf280a1c24b3d6335f87ee4319ca0443873`,
`vendors/veraPDF-library` @ `acfcc419a5df444e3e8b2a18266d01e249299957`,
`vendors/veraPDF-validation` @ `48b88fbeb8731c5489d9fc3f916bf91a22153508`.

pdfv pin: `3b8cede9965d105350497e7aff0f45159435872c`.

## Scope And Evidence

This review follows [verapdf-pdfv-core-drift-review.md](./verapdf-pdfv-core-drift-review.md) and focuses on meaningful user-visible parity gaps: rules that remain unbound, semantic families that exist but are too shallow, and verification gaps that can hide false parity.

Evidence was collected with:

```bash
make parity-profile-report
make parity-model-schema
make parity-corpus
```

The generated reports were read from `target/parity/profile-coverage.json`, `target/parity/model-schema.json`, `target/parity/unsupported-rules.json`, and `target/parity/corpus-agreement.json`.

## Current Parity Snapshot

pdfv now has broad model-family registration: 53 registered families, 555 registered properties, and 72 registered links. The registry includes document/catalog/page, resources, fonts, color spaces, XObjects, content streams, annotations/actions/forms, XMP-adjacent metadata, structure/accessibility, signatures, security, streams, and generic object families (`crates/core/src/validation.rs:1756`, `crates/core/src/validation.rs:1847`).

Across the 15 imported veraPDF profile files, pdfv currently sees 6,749 total rules, lowers 6,175 of them, and binds 5,038. Remaining unsupported reasons are:

| Reason | Count |
| --- | ---: |
| Missing model property | 1,095 |
| Unsupported expression | 574 |
| Missing semantic family | 42 |

The per-profile picture is uneven. PDF/A profiles bind only 25-33 rules each, while PDF/UA-2 and WTPDF bind more than 1,560 rules each because their generated profiles contain many currently bindable structure checks. This means aggregate bound-rule percentage overstates PDF/A readiness.

The Java-free corpus report currently has 9 rows, all matching expected outcomes, but it is still a narrow semantic smoke suite rather than a veraPDF agreement proof. The corpus report explicitly marks live oracle execution as disabled in normal parity runs (`crates/core/src/parity.rs:121`, `crates/core/src/parity.rs:129`).

## Meaningful Gaps

### 1. Rule binding is blocked more by derived properties than by missing families

The highest unsupported bucket is not missing object families; it is missing properties on families that already exist. The largest unsupported object buckets are `structureElement` 343, `font` 243, `metadata` 217, `object` 151, `document` 137, `annotation` 121, `colorSpace` 100, and `image` 87.

This matches the current implementation shape. Many pdfv model objects are dictionary-backed and expose direct keys or a few hand-derived values (`crates/core/src/validation.rs:5950`, `crates/core/src/validation.rs:5971`, `crates/core/src/validation.rs:6065`). veraPDF, by contrast, builds typed semantic adapters from parser objects, resources, graphics state, and cached model objects. For example, font creation branches by subtype and rendering context (`vendors/veraPDF-validation/validation-model/src/main/java/org/verapdf/gf/model/factory/fonts/FontFactory.java:59`), while color spaces branch across CalRGB, Device spaces, ICCBased, Lab, Separation, Indexed, DeviceN, and Pattern (`vendors/veraPDF-validation/validation-model/src/main/java/org/verapdf/gf/model/factory/colors/ColorSpaceFactory.java:81`).

Impact: profile XML can be present and families can be registered, yet important PDF/A and PDF/UA checks remain incomplete because properties like font symbolic status, embedded-file presence, XMP schema property details, image color-space presence, annotation appearance/action flags, and generic COS representation facts are not computed.

### 2. Expression grammar still drops hundreds of official rules

Unsupported expressions account for 574 rules. The dominant parser errors are `trailing expression input` and `expected expression`. Sample fragments include numeric comparisons, arithmetic, modulo-like bit checks, equality chains, and derived property comparisons. This is separate from model coverage: even when the target property eventually exists, the rule cannot execute until the bounded expression parser accepts the official profile syntax.

Impact: closing model properties alone will leave a hard ceiling on executable official profiles. This especially affects PDF/A rules using arithmetic relationships, flag masks, and compound boolean expressions.

### 3. Font, CMap, and embedded font semantics are too shallow for PDF/A parity

Font-related missing properties are one of the largest single clusters. veraPDF distinguishes Type 0, Type 1, TrueType, Type 3, CIDFont, descendant fonts, rendering mode, resources inside Type 3 fonts, CMap files, and embedded font programs (`vendors/veraPDF-validation/validation-model/src/main/java/org/verapdf/gf/model/factory/fonts/FontFactory.java:64`, `vendors/veraPDF-validation/feature-reporting/src/main/java/org/verapdf/features/gf/GFFeatureParser.java:804`). pdfv has registered `font`, `fontDescriptor`, `fontProgram`, `cMap`, and `embeddedFontFile` families, but many remain summary/dictionary facts rather than parsed font-program facts.

Impact: PDF/A font embedding, encoding, ToUnicode, symbolic/nonsymbolic, CMap, glyph-list, CIDSet, and Type 3 resource checks remain high-risk for false incomplete or false pass behavior.

### 4. Color, output-intent, graphics-state, and image semantics need resource-aware resolution

pdfv records content-stream resource uses and some color family facts (`crates/core/src/content.rs:558`, `crates/core/src/content.rs:647`, `crates/core/src/content.rs:660`, `crates/core/src/content.rs:685`). veraPDF mutates graphics state, resolves resources, carries current font/color/XObject state, and turns operators into typed validation objects (`vendors/veraPDF-validation/validation-model/src/main/java/org/verapdf/gf/model/factory/operators/OperatorParser.java:576`, `vendors/veraPDF-validation/validation-model/src/main/java/org/verapdf/gf/model/factory/operators/OperatorParser.java:658`).

Impact: checks around ICC profiles, output-intent color-space compatibility, annotation colors, image color spaces, soft masks, transparency, ExtGState transfer functions, patterns, shadings, and XObject transparency are not yet semantically equivalent to veraPDF.

### 5. XMP support is identification-oriented, not metadata-validation parity

pdfv's XMP module extracts bounded packets, namespace declarations, and PDF/A/PDF/UA/WTPDF identification claims (`crates/core/src/xmp.rs:1`, `crates/core/src/xmp.rs:81`). veraPDF's XMP core parses RDF into an XMP metadata tree, supports XPath expansion, property options, array handling, normalization, and serializer/utility behavior (`vendors/veraPDF-library/xmp-core/src/main/java/org/verapdf/xmp/impl/XMPMetaParser.java:72`, `vendors/veraPDF-library/xmp-core/src/main/java/org/verapdf/xmp/impl/XMPMetaImpl.java:83`).

Impact: metadata schema validation, prefix/namespace/property checks, packet byte facts, extension schema checks, repair parity, and richer feature reporting remain incomplete.

### 6. Accessibility exists, but WCAG-style semantic reconstruction is not yet deep enough

pdfv has an accessibility graph with structure root, role/class maps, parent tree, nodes, marked-content associations, object references, and artifacts (`crates/core/src/accessibility.rs:97`, `crates/core/src/accessibility.rs:313`). veraPDF's WCAG model layers semantic document/page/structure objects, repeated-character and list/table abstractions, and chunk parsing over page/XObject content (`vendors/veraPDF-validation/wcag-validation/src/main/java/org/verapdf/gf/model/impl/sa/GFSAPDFDocument.java:46`, `vendors/veraPDF-validation/wcag-validation/src/main/java/org/verapdf/gf/model/impl/sa/GFSAStructElem.java:48`, `vendors/veraPDF-validation/wcag-validation/src/main/java/org/verapdf/gf/model/factory/chunks/ChunkParser.java:520`).

Impact: PDF/UA and WTPDF structure rules can bind in bulk, but high-value accessibility behavior still needs stronger chunk text/image/annotation association, table/list semantics, repeated-character detection, optional-content visibility, and artifact handling.

### 7. Feature extraction follows the model gap

veraPDF feature reporting traverses catalog, names, actions, embedded files, AcroForm, output intents, page tree, annotations, resources, fonts, color spaces, ICC profiles, images, XObjects, patterns, shadings, signatures, security, and low-level document info (`vendors/veraPDF-validation/feature-reporting/src/main/java/org/verapdf/features/gf/GFFeatureParser.java:121`, `vendors/veraPDF-validation/feature-reporting/src/main/java/org/verapdf/features/gf/GFFeatureParser.java:147`, `vendors/veraPDF-validation/feature-reporting/src/main/java/org/verapdf/features/gf/GFFeatureParser.java:303`, `vendors/veraPDF-validation/feature-reporting/src/main/java/org/verapdf/features/gf/GFFeatureParser.java:848`). pdfv feature extraction is structurally useful, but its values are only as complete as the current validation model.

Impact: policy reports and extracted JSON/XML features can look broad while still missing veraPDF-grade facts inside each family.

### 8. Agreement verification is underpowered

The current corpus is deterministic and valuable for CI, but it has 9 Java-free rows and no live veraPDF oracle in normal runs. That is enough to catch report contract regressions, not enough to quantify feature parity over real-world or official corpora.

Impact: improvements can raise bound-rule counts without proving end-to-end agreement. Conversely, regressions in semantic interpretation may not be visible until a live veraPDF corpus run is added.

## Improvement Plan

1. **Make unsupported rules the primary backlog.** Treat `target/parity/unsupported-rules.json` as the work queue. Group by `primaryReason`, `objectType`, and property name. A change should name which unsupported cluster it reduces.

2. **Close expression coverage before broad semantic work.** Add bounded support for the official expression fragments now rejected as `trailing expression input` and `expected expression`: comparison edge cases, arithmetic, modulo/bit-mask forms, parenthesized compound booleans, and derived property references. Gate this with profile-coverage snapshots so unsupported-expression count falls monotonically.

3. **Promote generic dictionary exposure into typed derived facts.** Keep direct-key access, but add family-specific derivation layers for `object`, `document`, `metadata`, `annotation`, `image`, and `outputIntent`. This is the fastest route to reduce the 1,095 missing-property rules without overbuilding whole subsystems.

4. **Prioritize font/CMap/font-program semantics next.** Implement font subtype summaries, descriptor links, embedded font-file facts, ToUnicode/CMap facts, CIDSystemInfo, symbolic/nonsymbolic classification, width/CIDSet/glyph-list facts, and Type 3 resource summaries. This attacks the largest PDF/A-specific semantic cluster.

5. **Build effective resource resolution before deeper color/image checks.** Add resource inheritance and nested form/pattern/Type 3 contexts, then use content-stream `resourceUse` facts to resolve actual font, color-space, ExtGState, shading, and XObject targets. Do not add isolated color/image properties that cannot be tied back to effective resources.

6. **Replace XMP identification-only parsing with bounded RDF fact extraction.** Keep entity/external-resource hardening, but parse enough RDF/XMP structure to expose schema namespace, property, array, qualifier, extension-schema, and packet facts required by official metadata rules and repair reporting.

7. **Deepen accessibility after resource and content facts stabilize.** Use marked-content and resource resolution to improve text/image chunks, link/annotation associations, table/list/heading classification, repeated-character candidates, artifacts, and optional-content visibility. This should be tracked separately for PDF/UA and WTPDF because those profiles bind many rules but still need stronger semantic evidence.

8. **Expand parity verification in lockstep.** For each semantic family above, add generated fixtures, checked-in fixtures, feature-report assertions, and at least one live veraPDF corpus row when a corpus checkout is available. Keep Java-free CI deterministic, but publish milestone snapshots only after `make parity-profile-report`, `make parity-model-schema`, and `make parity-corpus` show the intended improvement.

## Recommended Phase Order

| Order | Workstream | Success metric |
| ---: | --- | --- |
| 1 | Expression parser parity | Unsupported-expression count drops materially from 574 without increasing missing-property count. |
| 2 | Derived property backlog | Missing-property count drops from 1,095, especially for `object`, `document`, `metadata`, `annotation`, `image`, and `outputIntent`. |
| 3 | Font/CMap/font-program semantics | `font` and `cMap` unsupported counts fall; PDF/A bound-rule counts improve beyond the current 25-33 range. |
| 4 | Effective resources plus color/image/XObject semantics | `colorSpace`, `image`, `extGState`, `formXObject`, `xObject`, and `iccProfile` unsupported counts fall with corpus fixtures proving resource resolution. |
| 5 | XMP RDF metadata facts | `metadata` unsupported count falls and XMP feature reports expose schema/property facts without dumping packet contents. |
| 6 | Accessibility semantic reconstruction | `structureElement` unsupported count falls and PDF/UA/WTPDF agreement fixtures cover chunks, tables, lists, links, artifacts, and optional content. |
| 7 | Feature and oracle parity snapshots | Feature extraction coverage and live veraPDF agreement rows grow with every semantic family milestone. |

## Bottom Line

The meaningful parity gap is no longer "pdfv lacks the profile files" or "pdfv lacks family names." The gap is executable semantics: official-expression coverage, derived model properties, resource-aware font/color/image resolution, full-enough XMP facts, deeper accessibility reconstruction, and corpus-backed agreement evidence.
