# Implementation Plan — Dependency-Ordered Build

Status: draft v1 · Owner: pdfv · Last updated: 2026-05-16

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

## 18. Phase 14 — M6 validation model breadth

| # | Task | Spec | Effort |
| --- | --- | --- | --- |
| 14.1 | Add internal model registry and schema checks connecting generated profile property/link references to model families. | 16, 17 | 3-5 days |
| 14.2 | Implement document/catalog/page/resource families, including AcroForm, structure-tree root, OC properties, language, permissions, outlines, names, and destinations. | 17, 70 | 1-2 weeks |
| 14.3 | Implement font/CMap and image/content model families with bounded content-stream summaries. | 15, 17, 70, 71 | 2-4 weeks |
| 14.4 | Implement annotation/action/form, color/transparency, structure/accessibility, and signature/security fact families. | 17, 70, 72 | 3-6 weeks |
| 14.5 | Add property-schema tests, cycle/cap tests, official-rule coverage tests, and performance benchmarks for broad traversal. | 17, 71, 72 | 1-2 weeks |

Exit criteria: generated built-in rules either bind to a known model property/link or have a tracked unsupported reason; broad model graph traversal remains iterative and bounded; official profile coverage improves measurably over Phase 13; fixtures cover every model family; standard gates and traversal benches pass.

## 19. Phase 15 — M6 XMP metadata and flavour detection

| # | Task | Spec | Effort |
| --- | --- | --- | --- |
| 15.1 | Implement bounded XMP packet extraction and namespace-aware RDF/XML parsing for identification schemas. | 18, 70 | 1-2 weeks |
| 15.2 | Map PDF/A, PDF/UA, and WTPDF claims to generated profiles with structured fallback and incompatibility warnings. | 16, 18, 20 | 3-5 days |
| 15.3 | Expose XMP facts through the validation model and report formats without dumping full metadata packets. | 17, 18, 20 | 3-5 days |
| 15.4 | Add auto-selection fixtures for absent, malformed, single-claim, multi-claim, incompatible, and encrypted-metadata cases. | 14, 18, 72 | 3-5 days |

Exit criteria: auto mode selects profiles from XMP claims when present; missing or malformed XMP yields structured warnings and default/fallback behaviour; reports show evidence for detected flavours; no XML entity/external resource path exists; standard gates plus strict XML hostile-input tests pass.

## 20. Phase 16 — M7 feature extraction and policy reports

| # | Task | Spec | Effort |
| --- | --- | --- | --- |
| 16.1 | Add `FeatureReport` data contracts and read-only feature extraction over the Phase 14 model families. | 10, 17, 19, 20 | 1-2 weeks |
| 16.2 | Add `pdfv validate --extract` and feature sections in JSON/XML reports. | 19, 50, 72 | 3-5 days |
| 16.3 | Complete a policy-language spike and implement a bounded first policy report format. | docs/research, 19, 70 | 1-2 weeks |
| 16.4 | Add policy report merging into JSON/XML and CLI `--policy-file`. | 19, 20, 50 | 3-5 days |

Exit criteria: feature extraction is read-only, bounded, and deterministic; policy reports consume feature reports rather than raw PDFs; CLI and library APIs expose feature/policy reports; standard gates pass.

## 21. Phase 17 — M7 metadata repair and report parity

| # | Task | Spec | Effort |
| --- | --- | --- | --- |
| 17.1 | Add `RepairReport` data contracts and explicit repair refusal model. | 10, 19, 20 | 2-4 days |
| 17.2 | Implement `pdfv repair-metadata` with non-in-place atomic writes, output directory validation, and prefix handling. | 19, 50, 70 | 1-2 weeks |
| 17.3 | Implement raw XML and static HTML report writers for validation/feature/policy/repair outputs. | 19, 20, 72 | 1-2 weeks |
| 17.4 | Publish CLI compatibility documentation for supported, intentionally different, and out-of-scope veraPDF flags. | 19, 50, 72 | 2-4 days |

Exit criteria: metadata repair never modifies inputs in place and removes failed outputs; raw and HTML reports pass golden tests; CLI docs explain deviations from veraPDF, including no literal password argument; standard gates pass.
