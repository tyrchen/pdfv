# Implementation Plan — Dependency-Ordered Build

Status: draft v1 · Owner: pdfv · Last updated: 2026-05-17

## 0. Readiness assessment

Ready:

- Prior-art study is complete: [../docs/research/study-verapdf-validator-architecture.md](../docs/research/study-verapdf-validator-architecture.md).
- Spec set defines library, parser, profiles, validation, reporting, CLI, security, performance, and testing contracts.
- Initial workspace exists.

Risks before broad implementation:

- Profile expression coverage needs a spike before promising wide PDF/A compatibility.
- Stream resource defaults need empirical validation against real PDFs.
- Profile source strategy needs a decision before large generator work.
- MRR/XML compatibility should remain out of M0 until demand is proven.

## 1. Why dependency order differs from feature order

The CLI is user-visible, but it lands after the library/report contracts because it must consume public APIs rather than private parser internals.

Profile breadth is valuable, but broad profile import lands after the parser facts, model graph, and rule IR exist; otherwise converted rules have no stable execution target.

Performance gates land after the parser spine is representative. Early microbenchmarks on throwaway parser shapes would create noise.

## 2. Estimated total effort

M0-M3 is approximately 13-23 developer-weeks for one developer. Parallelism helps with CLI/reporting and fixtures after the data model is stable, but parser/profile/engine work is mostly serial because each layer defines contracts for the next.

## 3. Phase 0 — risk retirement

| # | Deliverable | Lands in | Effort |
| --- | --- | --- | --- |
| 0.1 | Confirm latest dependency versions and pin `rust-toolchain.toml`. | 61 | 0.5 day |
| 0.2 | `spike-profile-expression-ir.md`: parse/evaluate representative veraPDF rule expressions into Rust IR. | docs/research | 2-4 days |
| 0.3 | `spike-pdf-stream-resource-limits.md`: validate stream decode caps and scan caps on sample PDFs. | docs/research | 1-2 days |
| 0.4 | Decide whether `apps/server` remains, is removed, or becomes `apps/cli`. | 61, 99 | 0.5 day |

Exit gate: spikes committed or explicitly deferred with M0 scope reduced; specs updated with findings.

## 4. Phase 1 — public contracts and workspace spine

| # | Task | Spec | Effort |
| --- | --- | --- | --- |
| 1.1 | Add `rust-toolchain.toml`, lint attributes, workspace dependency updates, and `apps/cli`. | 61 | 0.5-1 day |
| 1.2 | Implement public data model newtypes, options, reports, errors, and serde shape. | 10 | 2-3 days |
| 1.3 | Add JSON snapshot tests and doc examples for report/options. | 10, 20, 72 | 1 day |

Exit criteria: core crate builds with public contracts, docs compile, JSON snapshots pass, standard verification commands pass.

## 5. Phase 2 — parser M0

| # | Task | Spec | Effort |
| --- | --- | --- | --- |
| 2.1 | Implement byte/token parser for header, names, strings, numbers, arrays, dictionaries. | 11 | 3-5 days |
| 2.2 | Implement indirect objects, classic xref table, trailer, root/catalog access. | 11 | 3-5 days |
| 2.3 | Implement stream parsing with length checks, fallback scan, and parse facts. | 11, 70 | 2-4 days |
| 2.4 | Add malformed corpus tests and property tests for parser pieces. | 72 | 2-3 days |

Exit criteria: parser handles M0 fixtures, emits parse facts, respects limits, and does not panic on malformed corpus.

## 6. Phase 3 — profile IR and validation engine M0

| # | Task | Spec | Effort |
| --- | --- | --- | --- |
| 3.1 | Implement minimal built-in profile catalog and rule indexes. | 12 | 2-3 days |
| 3.2 | Implement bounded rule IR subset and evaluator. | 12 | 3-5 days |
| 3.3 | Implement model object wrappers for document/catalog/metadata/basic streams. | 13 | 3-5 days |
| 3.4 | Implement iterative traversal, deferred rule handling, counters, caps, and reports. | 13 | 3-5 days |

Exit criteria: `Validator::validate_reader` runs M0 rules end-to-end and returns complete `ValidationReport`.

## 7. Phase 4 — reporting and CLI M0

| # | Task | Spec | Effort |
| --- | --- | --- | --- |
| 4.1 | Implement JSON, pretty JSON, text report writers. | 20 | 1-2 days |
| 4.2 | Implement `pdfv validate`, `--format`, `--flavour`, `--profile`, `--max-failures`. | 50 | 2-3 days |
| 4.3 | Implement exit-code mapping and CLI integration tests. | 50, 72 | 1-2 days |

Exit criteria: closes M0 roadmap. A user can validate fixtures through CLI and library; all gates pass.

## 8. Phase 5 — M1 modern PDF coverage

| # | Task | Spec | Effort |
| --- | --- | --- | --- |
| 5.1 | Add xref streams and object streams. | 11 | 1-2 weeks |
| 5.2 | Add FlateDecode bounded streaming. | 11, 70 | 2-4 days |
| 5.3 | Expand model wrappers for pages, fonts, annotations, output intents. | 13 | 1-2 weeks |
| 5.4 | Add fuzz targets and performance measurement. | 71, 72 | 2-4 days |

Exit criteria: closes M1 roadmap with measured budgets.

## 9. Phase 6 — M2/M3 profile breadth and batch CLI

| # | Task | Spec | Effort |
| --- | --- | --- | --- |
| 6.1 | Build profile generator/importer based on Phase 0 decision. | 12 | 2-4 weeks |
| 6.2 | Implement custom profile XML loading. | 12, 70 | 3-5 days |
| 6.3 | Add recursive discovery, YAML config, bounded jobs, batch reports. | 50 | 1-2 weeks |
| 6.4 | Document JSON examples and conformance fixture matrix. | 20, 72 | 2-4 days |

Exit criteria: closes M2 and M3 roadmap.

## 10. Correctness of order

Contracts land before consumers: data model and error/report shapes precede parser, validator, and CLI.

Risk is retired before broad work: expression IR, stream limits, and profile source choices are resolved before profile breadth.

The smallest shippable slice stays end-to-end: M0 includes parser, profile, validation, report, and CLI, but constrains profile breadth.

## 11. Phase 7 — M4 compatibility report surface

| # | Task | Spec | Effort |
| --- | --- | --- | --- |
| 7.1 | Complete `spike-mrr-compatibility.md` and record the XML/MRR naming decision. | docs/research, 20, 50, 90, 99 | 0.5-1 day |
| 7.2 | Implement XML compatibility report writer for single and batch validation reports. | 20, 70, 72 | 1-2 days |
| 7.3 | Expose `pdfv validate --format xml` and deprecated `--format mrr` alias. | 50, 72 | 0.5 day |
| 7.4 | Update product-facing README and report examples. | 00, 20, 50, 72 | 0.5 day |

Exit criteria: `spike-mrr-compatibility.md` is published; XML report output is available from library and CLI; `mrr` maps to the same writer as a deprecated alias; tests cover single XML output and alias behavior; standard gates pass.

## 12. Phase 8 — M4 advanced profile facts

| # | Task | Spec | Effort |
| --- | --- | --- | --- |
| 8.1 | Promote the default built-in profile from M0 smoke checks to an M4 fact profile that also checks page, font, annotation, output-intent, and page content stream facts. | 12, 13, 90 | 0.5-1 day |
| 8.2 | Expose direct dictionary-backed model properties needed by imported veraPDF expressions where the existing model wrappers already own the dictionary. | 12, 13, 70 | 0.5-1 day |
| 8.3 | Add regression coverage for linked-object fact traversal, invalid feature facts, and CLI profile-list output. | 72 | 0.5 day |
| 8.4 | Update product examples and fixture matrix for the M4 default profile. | 20, 50, 72 | 0.5 day |

Exit criteria: default validation uses `pdfv-m4`; profile facts cover page contents/resources, font subtype, annotation subtype, output-intent destination profile, and content-stream length exposure; imported profile evaluation can read direct dictionary properties such as `Type`, `Subtype`, `Filter`, and `DestOutputProfile`; tests cover passing and failing linked-object facts; standard gates pass.

## 13. Phase 9 — M4 password/decryption support

| # | Task | Spec | Effort |
| --- | --- | --- | --- |
| 9.1 | Add redacted `PasswordSecret`, non-serializable validation option plumbing, and CLI password source resolution (`stdin`, file, env-var indirection; no literal argument). | 10, 14, 50, 70 | 1-2 days |
| 9.2 | Parse and validate Standard security handler encryption dictionaries for revisions 2-4, including `/CF`, `/StmF`, `/StrF`, and `/EncryptMetadata`. | 11, 14, 70 | 2-4 days |
| 9.3 | Implement password authentication and object-key-specific decryption for RC4 and AESV2 strings/streams with byte-counted limits. | 11, 14, 70 | 1-2 weeks |
| 9.4 | Wire decrypted parsing into validation/reporting so correct passwords produce normal validation and missing/wrong/unsupported passwords produce `ValidationStatus::Encrypted` with safe warnings. | 13, 14, 20, 50 | 2-4 days |
| 9.5 | Add encrypted fixture matrix, redaction tests, unsupported revision tests, fuzz/corpus cases, docs, and dependency audit/deny updates. | 14, 61, 70, 72 | 3-5 days |

Exit criteria: correct user and owner passwords validate supported RC4/AESV2 encrypted fixtures; strings and streams decrypt under resource limits; missing/wrong passwords and unsupported revisions return encrypted status and CLI exit 3 without secret leakage; password-bearing `Debug` is redacted by test; standard gates plus strict clippy, `cargo audit`, and `cargo deny check` pass.

## 14. Phase 10 — AES-256 decryption risk gate

| # | Deliverable | Lands in | Effort |
| --- | --- | --- | --- |
| 10.1 | Spike Standard security handler revisions 5-6, including Algorithm 2.A/2.B, `/OE`, `/UE`, `/Perms`, AESV3, and fixture availability. | docs/research, 14, 70 | 3-5 days |

Exit gate: either Phase 10 updates [14-password-decryption-design.md](./14-password-decryption-design.md) with implementation-ready AES-256 details and fixtures, or records revisions 5-6 as explicitly unsupported with a deferred finding.

## 15. Phase 11 — M4 AES-256 password decryption

| # | Task | Spec | Effort |
| --- | --- | --- | --- |
| 11.1 | Extend the Standard security handler to parse validated revision 5-6 dictionaries, including `/OE`, `/UE`, `/Perms`, `/CFM /AESV3`, and strict key-field lengths. | 11, 14, 70 | 1-2 days |
| 11.2 | Implement Algorithm 2.A/2.B authentication, owner/user password attempts, constant-time validation hash checks, AES-256-CBC file-key retrieval, and `/Perms` validation. | 14, 70 | 2-4 days |
| 11.3 | Add AESV3 string and stream decryption using the recovered file key directly under existing decrypted-byte limits. | 11, 14, 70 | 1-2 days |
| 11.4 | Add deterministic generated R5/R6 AESV3 fixtures covering user and owner passwords, wrong passwords, tampered `/Perms`, short key fields, and string/stream decryption. | 14, 72 | 1-2 days |
| 11.5 | Add `sha2` as a scoped decryption dependency and update audit/deny verification evidence. | 61, 70 | 0.5 day |

Exit criteria: generated R5 and R6 AESV3 fixtures validate with correct user and owner passwords; wrong passwords return `ValidationStatus::Encrypted` and CLI exit 3; tampered `/Perms` returns encrypted/unsupported before object decryption; AESV3 strings and streams decrypt under existing decrypted-byte caps; malformed short `/O`, `/U`, `/OE`, `/UE`, and `/Perms` fields parse-fail; standard gates plus strict clippy, `cargo audit`, and `cargo deny check` pass.

## 16. Phase 12 — M5 parser filter and source parity

| # | Task | Spec | Effort |
| --- | --- | --- | --- |
| 12.1 | Introduce `DecoderRegistry`, decoder trait, structured decode params, and per-filter error/fact reporting. | 15, 70 | 1-2 days |
| 12.2 | Implement bounded ASCIIHex, ASCII85, RunLength, Flate predictor, and LZW decoders with chained filter arrays. | 15, 70, 72 | 1-2 weeks |
| 12.3 | Extend `Crypt` handling so identity and named crypt filters compose correctly with Standard security handler decryption. | 14, 15, 70 | 2-4 days |
| 12.4 | Add configurable source storage for large files: memory threshold plus spill-file or optional mmap path. | 11, 15, 61, 70, 71 | 1-2 weeks |
| 12.5 | Implement `/Prev` xref-chain and hybrid-reference handling with structured facts for anomalies. | 11, 15, 70, 72 | 1-2 weeks |
| 12.6 | Add decoder, xref-chain, hostile-input, and performance regression fixtures. | 15, 71, 72 | 3-5 days |

Exit criteria: all required M5 text/image-neutral filters decode under byte caps; chained filters preserve order; xref `/Prev` chains and hybrid xrefs parse with deterministic facts; large-file parsing avoids eager full-file memory above threshold; unsupported image-pixel filters produce structured metadata-mode facts; standard gates plus strict clippy, `cargo audit`, `cargo deny check`, and parser benches pass.

## 17. Phase 13 — M5 profile catalog and expression parity

| # | Task | Spec | Effort |
| --- | --- | --- | --- |
| 13.1 | Add a Makefile-discoverable profile generator that reads vendored veraPDF XML profiles and emits deterministic Rust data. | 16, 61 | 3-5 days |
| 13.2 | Expand flavour model and CLI parsing for PDF/A-1/2/3/4, PDF/UA, and WTPDF built-ins. | 10, 16, 50 | 2-4 days |
| 13.3 | Extend the bounded expression parser/evaluator for nested paths, arithmetic, ternary, modulo, collection built-ins, and bounded regex checks needed by official profiles. | 12, 16, 70 | 1-2 weeks |
| 13.4 | Emit profile coverage metadata and make unsupported official rules first-class report data. | 16, 20, 72 | 2-4 days |
| 13.5 | Add per-profile load/list tests and coverage regression snapshots. | 16, 50, 72 | 3-5 days |

Exit criteria: `pdfv profiles list` shows every vendored built-in profile with source pin and executable coverage; generated data is deterministic; explicit profile selection loads the matching profile rather than always PDFA-1B; unsupported rules are reported with citations and force incomplete status; standard gates pass.

## 18. Phase 14 — M6 validation model registry and document/page foundation

| # | Task | Spec | Effort |
| --- | --- | --- | --- |
| 14.1 | Add internal model registry, `PropertySpec`, `LinkSpec`, object-family registration, and schema checks connecting generated profile references to model families. | 16, 17, 24 | 3-5 days |
| 14.2 | Implement document/catalog roots with metadata, page tree, names, outlines, destinations, AcroForm, OC properties, permissions, language, signature/security dictionary links, and deterministic context paths. | 17, 70 | 1-2 weeks |
| 14.3 | Implement page and annotation/action/form foundation families without deep content semantics: page dictionaries, inherited boxes, annotations, form fields, actions, additional actions, filespec references. | 17, 70, 72 | 1-2 weeks |
| 14.4 | Add model-schema parity metrics for object/property/link references and group unsupported rules by missing object/property/link. | 17, 24, 72 | 3-5 days |
| 14.5 | Add cycle/cap tests for page tree, names, outlines, destinations, annotations, forms, and action chains. | 17, 70, 72 | 3-5 days |

Exit criteria: generated built-in rules either bind to known registry entries or have a tracked unsupported reason; document/page/action/form traversal remains iterative and bounded; `make parity-model-schema` reports non-placeholder real counts; official profile bound-rule coverage improves measurably over Phase 13; standard gates and traversal benches pass.

## 19. Phase 15 — M6 content stream operator model

| # | Task | Spec | Effort |
| --- | --- | --- | --- |
| 15.1 | Implement bounded content stream tokenizer for operands/operators after stream decoding, with recoverable unknown-operator facts. | 15, 21, 70 | 1-2 weeks |
| 15.2 | Add operator facts for text object/state/show, marked content, graphics state, color, path, inline image, XObject invocation, compatibility, and unknown operators. | 21, 70, 72 | 2-4 weeks |
| 15.3 | Build lazy `ContentStreamSummary` caches for page, form XObject, pattern, and Type 3 charproc contexts. | 17, 21, 71 | 1-2 weeks |
| 15.4 | Register `ContentStream`, `Operator`, `MarkedContent`, and `InlineImage` model families and bind generated profile references to them. | 16, 17, 21, 24 | 3-5 days |
| 15.5 | Add fixtures and fuzz target for malformed streams, huge operand lists, marked content nesting, Type 3 charprocs, inline images, and unknown operators. | 21, 72 | 1 week |

Exit criteria: content-stream rules bind to registered operator schemas or receive named unsupported reasons; summaries are redacted, deterministic, bounded by `max_content_stream_ops`, and lazy; resource-use facts from `Tf`, color operators, `gs`, `sh`, and `Do` are available to Phase 16; standard gates plus content-stream fuzz smoke pass.

## 20. Phase 16 — M6 resource, font, color, and XObject semantics

| # | Task | Spec | Effort |
| --- | --- | --- | --- |
| 16.1 | Implement `EffectiveResources` with page-tree inheritance, nested form/pattern/type3 contexts, cycle detection, missing/wrong-type facts, and resource-use resolution. | 17, 21, 22, 70 | 1-2 weeks |
| 16.2 | Implement font and CMap summaries for Type0, Type1, TrueType, Type3, CIDFont, font descriptors, embedded font-file presence, ToUnicode, Encoding, widths, and CIDSystemInfo facts. | 22, 70, 72 | 2-4 weeks |
| 16.3 | Implement color/output-intent summaries for Device/Cal/Lab/Indexed/Separation/DeviceN/ICCBased spaces, ICC header facts, transparency groups, soft masks, and extGState references. | 22, 70, 72 | 2-4 weeks |
| 16.4 | Implement XObject, image, pattern, shading, and function summaries with links to decoded stream metadata and content summaries where applicable. | 21, 22, 71 | 1-3 weeks |
| 16.5 | Register resource/font/color/XObject model schemas and add parity metrics grouped by missing semantic family. | 16, 17, 22, 24 | 3-5 days |

Exit criteria: resource inheritance resolves content-stream uses deterministically; font/color/output-intent official rules move from missing-property unsupported reasons to bound or expression-unsupported reasons; fixture coverage exists for each resource/font/color family; embedded font/ICC bytes remain capped and report-safe; standard gates and resource traversal benches pass.

## 21. Phase 17 — M7 XMP metadata and flavour detection

| # | Task | Spec | Effort |
| --- | --- | --- | --- |
| 17.1 | Implement bounded XMP packet extraction and namespace-aware RDF/XML parsing for identification schemas. | 18, 70 | 1-2 weeks |
| 17.2 | Map PDF/A, PDF/UA, and WTPDF claims to generated profiles with structured fallback and incompatibility warnings. | 16, 18, 20 | 3-5 days |
| 17.3 | Expose XMP facts through the validation model and report formats without dumping full metadata packets. | 17, 18, 20 | 3-5 days |
| 17.4 | Add auto-selection fixtures for absent, malformed, single-claim, multi-claim, incompatible, and encrypted-metadata cases. | 14, 18, 72 | 3-5 days |

Exit criteria: auto mode selects profiles from XMP claims when present; missing or malformed XMP yields structured warnings and default/fallback behaviour; reports show evidence for detected flavours; no XML entity/external resource path exists; standard gates plus strict XML hostile-input tests pass.

## 22. Phase 18 — M7 structure and accessibility semantics

| # | Task | Spec | Effort |
| --- | --- | --- | --- |
| 18.1 | Implement bounded structure tree traversal for `/StructTreeRoot`, `/K`, `/RoleMap`, `/ClassMap`, `/IDTree`, `/ParentTree`, and page associations. | 17, 23, 70 | 2-4 weeks |
| 18.2 | Associate marked-content `MCID` facts from Phase 15 with structure elements, pages, annotations, images, artifacts, and content chunks. | 21, 22, 23 | 2-4 weeks |
| 18.3 | Implement accessibility semantic families: tagged document, structure element, text chunk, image chunk, annotation/link, table/list/heading, artifact, and repeated-character candidate facts. | 23, 72 | 3-6 weeks |
| 18.4 | Register accessibility schemas and group PDF/UA/WTPDF unsupported rules by missing semantic family. | 16, 17, 23, 24 | 3-5 days |
| 18.5 | Add generated and corpus fixtures for tagged/untagged docs, role maps, parent-tree cycles, artifacts, links, image alt text, lists, tables, and malformed MCID references. | 23, 72 | 1-2 weeks |

Exit criteria: PDF/UA and WTPDF rules that depend on structure/marked-content semantics bind to known schemas or have named semantic-family unsupported reasons; accessibility graph construction is lazy, bounded, and text-redacted by default; profile coverage improves measurably for PDF/UA/WTPDF profiles; standard gates plus structure fuzz smoke pass.

## 23. Phase 19 — M7 parity metrics and corpus agreement

| # | Task | Spec | Effort |
| --- | --- | --- | --- |
| 19.1 | Add Makefile targets `parity-profile-report`, `parity-model-schema`, and `parity-corpus`. | 24, 61, 72 | 1-2 days |
| 19.2 | Emit deterministic `target/parity/profile-coverage.json`, `model-schema.json`, `unsupported-rules.json`, and `corpus-agreement.json`. | 16, 17, 24 | 3-5 days |
| 19.3 | Add review-gated coverage decrease detection and milestone snapshot instructions. | 24, 72, 93 | 2-4 days |
| 19.4 | Expand checked-in/generated semantic corpus rows for parser, profile, operator, resource/font/color, XMP, and accessibility families. | 21, 22, 23, 24, 72 | 1-2 weeks |

Exit criteria: parity reports contain non-placeholder real counts; unsupported rules have exactly one primary reason; bound-rule and model-family coverage can be compared across phases; normal CI can run generated/checked-in corpus rows without Java; live veraPDF oracle rows remain opt-in; standard gates pass.

## 24. Phase 20 — M8 feature extraction and policy reports

| # | Task | Spec | Effort |
| --- | --- | --- | --- |
| 20.1 | Add `FeatureReport` data contracts and read-only feature extraction over document/page/operator/resource/font/color/XMP/accessibility model families. | 10, 17, 19, 20, 21, 22, 23 | 1-2 weeks |
| 20.2 | Add `pdfv validate --extract` and feature sections in JSON/XML reports. | 19, 50, 72 | 3-5 days |
| 20.3 | Complete a policy-language spike and implement a bounded first policy report format. | docs/research, 19, 70 | 1-2 weeks |
| 20.4 | Add policy report merging into JSON/XML and CLI `--policy-file`. | 19, 20, 50 | 3-5 days |

Exit criteria: feature extraction is read-only, bounded, and deterministic; policy reports consume feature reports rather than raw PDFs; CLI and library APIs expose feature/policy reports; standard gates pass.

## 25. Phase 21 — M8 metadata repair and report parity

| # | Task | Spec | Effort |
| --- | --- | --- | --- |
| 21.1 | Add `RepairReport` data contracts and explicit repair refusal model. | 10, 19, 20 | 2-4 days |
| 21.2 | Implement `pdfv repair-metadata` with non-in-place atomic writes, output directory validation, and prefix handling. | 19, 50, 70 | 1-2 weeks |
| 21.3 | Implement raw XML and static HTML report writers for validation/feature/policy/repair outputs. | 19, 20, 72 | 1-2 weeks |
| 21.4 | Publish CLI compatibility documentation for supported, intentionally different, and out-of-scope veraPDF flags. | 19, 50, 72 | 2-4 days |

Exit criteria: metadata repair never modifies inputs in place and removes failed outputs; raw and HTML reports pass golden tests; CLI docs explain deviations from veraPDF, including no literal password argument; standard gates pass.

## 26. Phase 22 — M9 veraPDF-grade readiness

Detailed tasks live in [28-verapdf-grade-validation-impl-plan.md](./28-verapdf-grade-validation-impl-plan.md). This phase is intentionally separated from the base engine plan because it is driven by unsupported-rule clusters and live oracle agreement rather than by component construction.

| # | Task | Spec | Effort |
| --- | --- | --- | --- |
| 22.1 | Produce the baseline unsupported-rule burn-down snapshot and classify release-blocking clusters. | 25, 26 | 3-7 days |
| 22.2 | Close expression, derived-property, font/CMap, resource/color/image/XObject, XMP RDF, and accessibility clusters in the order defined by the readiness plan. | 26, 28 | 12-24 weeks |
| 22.3 | Add live veraPDF oracle corpus execution, manifests, agreement summaries, and drift classification. | 27, 28 | 4-8 weeks |
| 22.4 | Publish readiness release snapshots and documentation with supported profiles, corpus evidence, and known drift. | 25, 27, 28 | 1 week |

Exit criteria: all release gates in [25-verapdf-grade-validation-prd.md](./25-verapdf-grade-validation-prd.md#8-release-gate) pass; the oracle gates in [27-oracle-corpus-verification-plan.md](./27-oracle-corpus-verification-plan.md#6-release-gates) pass; standard repository gates pass.
