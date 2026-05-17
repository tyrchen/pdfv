# Review: veraPDF Vendor vs pdfv Core Drift

Status: Done · Owner: pdfv · Date: 2026-05-16

Vendor pins:
`vendors/veraPDF-parser` @ `5164caf280a1c24b3d6335f87ee4319ca0443873`,
`vendors/veraPDF-library` @ `acfcc419a5df444e3e8b2a18266d01e249299957`,
`vendors/veraPDF-validation` @ `48b88fbeb8731c5489d9fc3f916bf91a22153508`,
`vendors/veraPDF-apps` @ `c6f3531d81b203e0426e63d60a4080b91b219dd0`.

pdfv pin: `2b252c0f6aac4f494f7daaba5430252d3caec9cb`.

## Scope And Method

This review compares core functionality, not legal notices, build metadata, fixtures, generated lockfiles, or binary assets. LoC was measured with `tokei` on 2026-05-16.

Measurement commands:

```bash
tokei vendors --exclude target --exclude '*.pdf' --exclude '*.png' --exclude '*.jpg' --exclude '*.jar' --exclude '.git'
tokei crates apps --exclude target
```

For module-level implementation rows, the review uses production source directories and excludes tests, GUI/installer packaging, and generated profile XML. Generated validation profiles are listed separately because they are rule/catalog data, not hand-written engine capability.

## Headline

veraPDF has roughly **155k physical Java lines** in the vendored tree, with **95.2k Java code lines** and another **90.7k XML/XSL code lines**. Its production validation stack has about **86.8k Java code lines** across parser, library, validation model, feature reporting, metadata repair, WCAG/accessibility, and CLI modules.

pdfv has **21.5k physical Rust lines**, **20.1k Rust code lines**, and **81.8k generated profile XML lines**. pdfv has imported much of the profile data shape, but the engine/model surface remains far smaller than veraPDF's Java parser and validation-model surface.

## Core Feature Inventory

### veraPDF Vendor Set

- **Byte-level and COS parsing**: seekable PDF parsing, header/xref/trailer recovery, object streams, xref streams, stream length recovery, COS object model, filters, IO abstractions, and parser warnings. Key anchors include `PDFParser`, `SeekableCOSParser`, `COSDocument`, and `COSObject` (`vendors/veraPDF-parser/src/main/java/org/verapdf/parser/PDFParser.java:45`, `vendors/veraPDF-parser/src/main/java/org/verapdf/parser/SeekableCOSParser.java:40`, `vendors/veraPDF-parser/src/main/java/org/verapdf/cos/COSDocument.java:49`, `vendors/veraPDF-parser/src/main/java/org/verapdf/cos/COSObject.java:34`).
- **PDF semantic object model**: PD-level document, page tree, resources, annotations, actions, colors, encryption, fonts, forms, functions, images, optional content, patterns, and structure tree (`vendors/veraPDF-parser/src/main/java/org/verapdf/pd/PDDocument.java:45`, `vendors/veraPDF-parser/src/main/java/org/verapdf/pd/PDPage.java:38`, `vendors/veraPDF-parser/src/main/java/org/verapdf/pd/PDResources.java:43`, `vendors/veraPDF-parser/src/main/java/org/verapdf/pd/structure/PDStructTreeRoot.java:33`).
- **Standard security handler**: password-based decryption support in the parser stack (`vendors/veraPDF-parser/src/main/java/org/verapdf/pd/encryption/StandardSecurityHandler.java:47`).
- **Processor/library facade**: processor configs, item/batch processors, report handlers, task routing, app config, and result models (`vendors/veraPDF-library/core/src/main/java/org/verapdf/processor/ProcessorFactory.java:50`, `vendors/veraPDF-library/core/src/main/java/org/verapdf/processor/ProcessorImpl.java:58`).
- **Profile-driven validation**: profile directories, rules, JavaScript expression evaluation, iterative validator traversal, rule summaries, and pass/fail reporting (`vendors/veraPDF-library/core/src/main/java/org/verapdf/pdfa/validation/profiles/ValidationProfile.java:38`, `vendors/veraPDF-library/core/src/main/java/org/verapdf/pdfa/validation/profiles/Rule.java:39`, `vendors/veraPDF-library/core/src/main/java/org/verapdf/pdfa/validation/validators/BaseValidator.java:53`, `vendors/veraPDF-library/core/src/main/java/org/verapdf/pdfa/validation/validators/JavaScriptEvaluator.java:35`).
- **Greenfield validation model binding**: foundry registration, parser-to-model adapter, document wrappers, COS wrappers, PD wrappers, font/color/function factories, graphics/content stream operator model, static per-document containers, and visitors (`vendors/veraPDF-validation/validation-model/src/main/java/org/verapdf/gf/foundry/VeraFoundry.java:40`, `vendors/veraPDF-validation/validation-model/src/main/java/org/verapdf/gf/model/GFModelParser.java:63`, `vendors/veraPDF-validation/validation-model/src/main/java/org/verapdf/gf/model/impl/pd/GFPDDocument.java:51`, `vendors/veraPDF-validation/validation-model/src/main/java/org/verapdf/gf/model/factory/operators/OperatorFactory.java:41`).
- **Feature reporting**: adapters for metadata, annotations, output intents, patterns, forms, graphics state, properties dictionaries, signatures, images, low-level info, outlines, document security, embedded files, fonts, pages, ICC profiles, PostScript XObjects, info dictionary, color spaces, interactive form fields, actions, and shadings (`vendors/veraPDF-validation/feature-reporting/src/main/java/org/verapdf/features/gf/GFFeatureParser.java:62`, `vendors/veraPDF-validation/feature-reporting/src/main/java/org/verapdf/features/gf/GFFeaturesObjectCreator.java:47`).
- **Metadata repair**: Greenfield metadata fixer implementation and model/schema adapters (`vendors/veraPDF-validation/metadata-fixer/src/main/java/org/verapdf/metadata/fixer/gf/MetadataFixerImpl.java:63`).
- **Accessibility/WCAG semantic model**: semantic structure elements, pages, chunks, annotations, tables, repeated characters, and content-stream-derived accessibility objects (`vendors/veraPDF-validation/wcag-validation/src/main/java/org/verapdf/gf/model/impl/sa/GFSAPDFDocument.java:46`, `vendors/veraPDF-validation/wcag-validation/src/main/java/org/verapdf/gf/model/impl/sa/GFSAStructElem.java:48`).
- **Full XMP support**: a dedicated XMP core module with parsing, properties, options, XPath, and implementation helpers under `vendors/veraPDF-library/xmp-core/src/main/java`.
- **CLI and app packaging**: CLI arg parsing, Greenfield wrapper, multi-thread processing, report format selection, Docker packaging, installer and GUI surfaces (`vendors/veraPDF-apps/cli/src/main/java/org/verapdf/cli/VeraPdfCli.java:56`, `vendors/veraPDF-apps/cli/src/main/java/org/verapdf/cli/commands/VeraCliArgParser.java:68`, `vendors/veraPDF-apps/cli/src/main/java/org/verapdf/cli/multithread/MultiThreadProcessor.java:43`).

### pdfv

- **Library-first public API and report model**: public error/config/report types, resource limits, validation report, output formats, and metadata repairer live mainly in `crates/core/src/lib.rs` (`crates/core/src/lib.rs:570`, `crates/core/src/lib.rs:758`, `crates/core/src/lib.rs:1028`, `crates/core/src/lib.rs:2096`).
- **Bounded tolerant parser**: seekable source handling, memory/spill-file source storage, resource limits, header/xref/trailer/object parsing, stream decoding, and parser facts (`crates/core/src/parser.rs:46`, `crates/core/src/parser.rs:79`, `crates/core/src/parser.rs:921`, `crates/core/src/parser.rs:1792`).
- **Stream filters and decryption**: pluggable stream decoder registry plus built-in decoders; optional Standard security handler support is implemented in Rust (`crates/core/src/parser.rs:135`, `crates/core/src/parser.rs:1226`, `crates/core/src/parser/encryption.rs:213`).
- **Profile import and bounded rule IR**: built-in profile repository, custom profile gate, XML profile import, bounded rule expressions, and default rule evaluator (`crates/core/src/profile.rs:27`, `crates/core/src/profile.rs:37`, `crates/core/src/profile.rs:393`, `crates/core/src/profile.rs:561`, `crates/core/src/profile.rs:1293`).
- **Validation session/model graph**: validator facade, model object graph, selected object families, rule execution, unsupported-rule reporting, feature extraction, and policy evaluation (`crates/core/src/validation.rs:535`, `crates/core/src/validation.rs:593`, `crates/core/src/validation.rs:1067`).
- **Bounded XMP identification**: XMP packet extraction, namespace capture, PDF/A, PDF/UA, and WTPDF flavour detection from catalog metadata (`crates/core/src/xmp.rs:1`, `crates/core/src/xmp.rs:91`).
- **CLI**: `validate`, `repair-metadata`, and `profiles list`; YAML config; jobs; recursive discovery; password sources; report formats; feature extraction; policy files; stable exit codes (`apps/cli/src/main.rs:56`, `apps/cli/src/main.rs:63`, `apps/cli/src/main.rs:90`, `apps/cli/src/main.rs:149`, `apps/cli/src/main.rs:178`).
- **Tests and conformance harness**: Rust unit/integration tests plus an ignored veraPDF-corpus test harness (`apps/cli/tests/validate.rs`, `apps/cli/tests/verapdf_corpus.rs`, `crates/core/tests/encrypted.rs`, `crates/core/tests/fixtures.rs`).

## LoC Comparison

### Top-Level Measurement

| Codebase | Measured scope | Files | Physical lines | Code lines | Comments | Blanks |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| veraPDF vendors | `vendors`, excluding target/binary assets | 1,384 | 260,269 | 190,406 | 51,270 | 18,593 |
| veraPDF Java only | all vendored Java | 1,158 | 155,226 | 95,210 | 42,971 | 17,045 |
| pdfv workspace source | `crates apps`, excluding target | 33 | 103,428 | 102,017 | 4 | 1,407 |
| pdfv Rust only | all Rust in `crates apps` | 14 | 21,501 | 20,102 | 0 | 1,399 |

The `pdfv workspace source` total is dominated by generated profile XML. The implementation comparison should use `pdfv Rust only` unless the question is profile-data coverage.

### Production Implementation Modules

| Area | veraPDF production code lines | pdfv production code lines | Approx. ratio | Functional note |
| --- | ---: | ---: | ---: | --- |
| Parser/COS/PD model | 30,028 | 4,618 | 6.5x | pdfv has a bounded COS parser and decryption, but veraPDF has a broad PD semantic model for fonts, colors, resources, annotations, structure, actions, images, optional content, patterns, and functions. |
| Validation library facade, profiles, reports | 20,038 | 6,170 | 3.2x | pdfv has the core facade, reports, profile import, and bounded IR; veraPDF has mature profile directories, JavaScript expression execution, processor tasks, multi-format handlers, and policy/report models. |
| Validation-model binding | 15,210 | 3,960 | 3.8x | pdfv has selected model objects and links; veraPDF has a wide Greenfield adapter layer for COS/PD/operator/font/color/function objects. |
| XMP handling | 10,385 | 879 | 11.8x | pdfv does bounded identification/flavour detection; veraPDF has a full XMP core. |
| Feature reporting | 3,618 | included in 3,960 | n/a | pdfv extracts bounded feature/policy data; veraPDF has object-family-specific adapters for many PDF feature families. |
| Metadata repair | 1,209 | included in 3,354 | n/a | pdfv has safe repair output flow; veraPDF has broader schema/model adapters. |
| WCAG/accessibility semantic model | 4,628 | thin structure exposure inside 3,960 | high | This is one of the largest product-scope drifts. |
| CLI/app shell | 1,692 | 1,167 | 1.4x | CLI size is close; drift is mostly in GUI/installer/Docker and multi-process packaging, not basic CLI validation. |
| Generated validation profiles | 85,751 XML code lines | 81,844 XML code lines | 1.0x | Profile data volume is close because pdfv embeds imported/generated profile XML. Executable rule coverage is not equivalent to data volume. |

### pdfv Rust File Breakdown

| pdfv file/scope | Code lines | Core functionality |
| --- | ---: | --- |
| `crates/core/src/lib.rs` | 3,354 | Public types, errors, configs, report writing, metadata repair types. |
| `crates/core/src/parser.rs` | 3,459 | Bounded parser, COS objects, xref/trailer/object/stream handling, stream filters. |
| `crates/core/src/parser/encryption.rs` | 1,159 | Standard security handler support. |
| `crates/core/src/profile.rs` | 2,816 | Profile repository, profile import, rule IR, rule evaluator. |
| `crates/core/src/validation.rs` | 3,960 | Validator, model graph, rule execution, feature/policy reports. |
| `crates/core/src/xmp.rs` | 879 | XMP packet parsing and flavour detection. |
| `apps/cli/src/main.rs` | 1,167 | CLI commands, config mapping, discovery, jobs, output, exit codes. |
| tests/benches/examples | 3,208 | Integration tests, encrypted fixtures, corpus harness, benchmark, profile generator. |
| `crates/core/src/generated_profiles.rs` | 100 | Generated profile source registry. |

## Core Drift

1. **Parser semantic breadth is the largest implementation drift.** pdfv can parse and retain conformance facts, but veraPDF's parser includes a much larger PD layer for fonts, colors, resources, page tree details, annotations, actions, forms, functions, images, optional content, patterns, signatures, and structure. This affects rule coverage because many official rules need semantic objects, not only COS dictionaries.

2. **Validation-model coverage is broader in veraPDF.** veraPDF has explicit Greenfield wrappers for model object families and content stream operators. pdfv has a deliberately smaller model graph, so imported profile data can exist while many rule properties still resolve to unsupported or absent model facts.

3. **Rule-data volume is close, executable rule parity is not.** pdfv's generated profile XML volume nearly matches veraPDF's core XML resources, but pdfv lowers expressions into a bounded Rust IR. Any JavaScript expression, model property, function, or object link outside that IR/model remains drift even when the XML profile is present.

4. **Content stream/operator semantics are thin in pdfv.** veraPDF models graphics operators, text operators, marked content, inline images, Type 3 font operators, color operators, clipping, path construction, path painting, and XObject invocation. pdfv has parser-level stream handling and selected feature exposure, not a comparable operator semantic layer.

5. **Fonts, colors, and external resources are high-risk gaps.** The vendor parser and validation model both have dedicated factories and wrappers for fonts, color spaces, functions, ICC profiles, images, and resources. pdfv currently has selected dictionary/property exposure. This will limit PDF/A and PDF/UA rule execution most sharply around output intents, font embedding/encoding, color management, image/XObject validation, and resource inheritance.

6. **Accessibility/WCAG is product-scope drift, not a small missing rule.** veraPDF has a separate WCAG validation model with semantic accessibility objects, chunks, pages, tables, annotations, and structure elements. pdfv's current model exposes some structure-related dictionaries but does not reconstruct comparable semantic accessibility content.

7. **XMP depth differs.** pdfv handles bounded XMP identification and flavour selection. veraPDF carries a full XMP core and richer metadata/fixer integration. This matters for metadata validation and repair beyond simple PDF/A/PDF/UA/WTPDF identification claims.

8. **Feature reporting is partial in pdfv.** veraPDF has a broad object-family feature reporting subsystem. pdfv has feature and policy report capability, but the underlying extracted facts are far narrower and tied to current model coverage.

9. **CLI drift is modest.** pdfv already covers the essential validation CLI concerns: formats, profile selection, batch discovery, jobs, config, password sources, feature extraction, policy files, metadata repair command, and stable exit codes. Remaining CLI drift is mostly compatibility details, GUI/installer/Docker packaging, and exact veraPDF output contracts.

10. **Architecture drift is intentional in some places.** pdfv avoids JavaScript-in-process evaluation, global foundry registration, thread-local static document state, and unbounded parsing. These are not gaps to close directly; they are Rust/security design choices. The drift to close is capability parity through explicit session-owned state, typed model facts, and bounded evaluators.

## Practical Implications

The next parity work should not start by adding more CLI flags. The CLI is already close enough for the current product surface. The highest leverage work is to expand the validation model beneath the existing facade:

1. Add model/property coverage where imported profiles already produce unsupported rules.
2. Prioritize font, color/output intent, resource inheritance, page tree, annotations/actions, structure tree, and XMP metadata properties.
3. Add a content-stream operator model only where it unlocks concrete profile rules, starting with text/marked-content and graphics-state facts needed by PDF/UA and PDF/A rules.
4. Keep parser anomalies as facts, not only errors, because conformance rules need malformed-but-parseable evidence.
5. Treat WCAG/accessibility as a separate milestone with its own design, because veraPDF's WCAG module is a semantic reconstruction subsystem, not just profile XML.

## Recommended Drift Metrics

Track these metrics in future phase reviews:

| Metric | Why it matters |
| --- | --- |
| Imported profile rules by flavour | Shows data coverage. |
| Executable rules by flavour | Shows IR/function coverage. |
| Unsupported rules grouped by missing object type/property/function | Points directly to model work. |
| Model object families implemented vs vendor families | Prevents over-counting profile XML as validator capability. |
| Feature families with semantic parity fixtures | Tracks user-visible feature reporting drift. |
| Corpus pass/fail agreement with veraPDF | Captures end-to-end behavior once model coverage improves. |

## Bottom Line

pdfv has a strong library/CLI skeleton, bounded parser, generated profile data, rule IR, report contracts, and safety posture. The core drift is that veraPDF has a much richer PDF semantic model and validation binding layer. Closing parity requires model and rule-execution depth, especially around fonts, color management, resources, structure/accessibility, XMP, and content streams; it does not primarily require more profile files or more CLI surface.
