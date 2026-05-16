# Implementation Plan — Dependency-Ordered Build

Status: draft v1 · Owner: pdfv · Last updated: 2026-05-15

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
