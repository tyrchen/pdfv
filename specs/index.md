# pdfv Spec Index

Status: draft v1 · Owner: pdfv · Last updated: 2026-05-16

## Reading order

Read in this order when implementing:

1. [00-prd.md](./00-prd.md) — product requirements and success criteria.
2. [10-data-model.md](./10-data-model.md) — public report/config/profile data contracts.
3. [11-parser-core-design.md](./11-parser-core-design.md) — tolerant COS/PD parser and parse facts.
4. [12-profile-rule-ir-design.md](./12-profile-rule-ir-design.md) — profile loading and bounded rule expression IR.
5. [13-validation-engine-design.md](./13-validation-engine-design.md) — validation session, model graph, traversal, diagnostics.
6. [14-password-decryption-design.md](./14-password-decryption-design.md) — password input, encryption dictionary parsing, and scoped decryption.
7. [15-parser-filter-source-parity-design.md](./15-parser-filter-source-parity-design.md) — stream filters, source storage, and xref-chain parity.
8. [16-profile-catalog-rule-parity-design.md](./16-profile-catalog-rule-parity-design.md) — full built-in profile catalog and rule-expression parity.
9. [17-validation-model-parity-design.md](./17-validation-model-parity-design.md) — broad validation model families and property/link schema.
10. [18-xmp-metadata-flavour-design.md](./18-xmp-metadata-flavour-design.md) — XMP parsing and PDF/A/PDF/UA/WTPDF auto flavour detection.
11. [19-verapdf-product-surface-parity-design.md](./19-verapdf-product-surface-parity-design.md) — feature extraction, policy, repair, raw/HTML reports, and CLI parity surfaces.
12. [20-reporting-design.md](./20-reporting-design.md) — JSON/text/XML output and batch summaries.
13. [50-cli-design.md](./50-cli-design.md) — CLI UX, config, exit codes, concurrency.
14. Cross-cuts: [61-crates-and-features.md](./61-crates-and-features.md), [70-security.md](./70-security.md), [71-performance-budgets.md](./71-performance-budgets.md), [72-testing-strategy.md](./72-testing-strategy.md).
15. [80-glossary.md](./80-glossary.md), [90-roadmap.md](./90-roadmap.md), [91-impl-plan.md](./91-impl-plan.md), [93-improvements-review.md](./93-improvements-review.md), [99-key-decisions.md](./99-key-decisions.md).

## Build-order graph

```text
00-prd
  -> 10-data-model
      -> 11-parser-core-design
          -> 12-profile-rule-ir-design
              -> 13-validation-engine-design
                  -> 14-password-decryption-design
                      -> 15-parser-filter-source-parity-design
                          -> 16-profile-catalog-rule-parity-design
                              -> 17-validation-model-parity-design
                                  -> 18-xmp-metadata-flavour-design
                                      -> 19-verapdf-product-surface-parity-design
                                          -> 20-reporting-design
                                              -> 50-cli-design

61-crates-and-features, 70-security, 71-performance-budgets, 72-testing-strategy
  constrain every implementation phase.

80-glossary, 90-roadmap, 91-impl-plan, 93-improvements-review, 99-key-decisions
  explain terms, stakeholder milestones, engineering order, and load-bearing choices.
```

## Spec table

| File | Type | Purpose |
| --- | --- | --- |
| [00-prd.md](./00-prd.md) | PRD | Defines users, goals, non-goals, success metrics, and naming. |
| [10-data-model.md](./10-data-model.md) | Data model | Defines public Rust types, JSON shape, profile shape, parse facts, warnings, and invariants. |
| [11-parser-core-design.md](./11-parser-core-design.md) | Component design | Designs the safe, tolerant PDF parser and COS/PD object model. |
| [12-profile-rule-ir-design.md](./12-profile-rule-ir-design.md) | Component design | Designs built-in/custom profile handling and bounded expression evaluation. |
| [13-validation-engine-design.md](./13-validation-engine-design.md) | Component design | Designs `ValidationSession`, graph traversal, diagnostics, and library entrypoints. |
| [14-password-decryption-design.md](./14-password-decryption-design.md) | Component design | Designs scoped password input and Standard security handler decryption support. |
| [15-parser-filter-source-parity-design.md](./15-parser-filter-source-parity-design.md) | Component design | Designs stream filter coverage, source storage, and xref-chain parity with veraPDF. |
| [16-profile-catalog-rule-parity-design.md](./16-profile-catalog-rule-parity-design.md) | Component design | Designs full built-in profile catalog generation and bounded rule-expression parity. |
| [17-validation-model-parity-design.md](./17-validation-model-parity-design.md) | Component design | Designs broad validation model families, property/link schemas, and lazy model caches. |
| [18-xmp-metadata-flavour-design.md](./18-xmp-metadata-flavour-design.md) | Component design | Designs XMP packet parsing, metadata facts, and PDF/A/PDF/UA/WTPDF auto flavour detection. |
| [19-verapdf-product-surface-parity-design.md](./19-verapdf-product-surface-parity-design.md) | Component design | Designs feature extraction, policy reports, metadata repair, raw/HTML reports, and CLI parity surfaces. |
| [20-reporting-design.md](./20-reporting-design.md) | Component design | Designs report formatting, summaries, and stable JSON/text output. |
| [50-cli-design.md](./50-cli-design.md) | CLI design | Defines CLI commands, config loading, exit codes, and bounded parallelism. |
| [61-crates-and-features.md](./61-crates-and-features.md) | Workspace design | Defines crate layout, feature flags, and dependency policy. |
| [70-security.md](./70-security.md) | Cross-cut | Threat model, resource limits, hostile input handling, and unsafe policy. |
| [71-performance-budgets.md](./71-performance-budgets.md) | Cross-cut | Performance budgets, allocation rules, benchmarks, and CI gates. |
| [72-testing-strategy.md](./72-testing-strategy.md) | Cross-cut | Test pyramid, fixtures, fuzzing, conformance suites, and regression rules. |
| [80-glossary.md](./80-glossary.md) | Glossary | Defines overloaded PDF, validation, and product terms. |
| [90-roadmap.md](./90-roadmap.md) | Roadmap | Stakeholder-facing milestones and exit criteria. |
| [91-impl-plan.md](./91-impl-plan.md) | Implementation plan | Engineer-facing dependency order, phases, effort, and gates. |
| [93-improvements-review.md](./93-improvements-review.md) | Review backlog | Deferred findings and non-blocking architecture improvements from implementation reviews. |
| [99-key-decisions.md](./99-key-decisions.md) | Key decisions | Permanent design decisions with alternatives and rationale. |

## Research anchors

- [../docs/research/study-verapdf-validator-architecture.md](../docs/research/study-verapdf-validator-architecture.md) — veraPDF parser, profile, validator, reporting, and CLI architecture. The spec adopts its library-first facade, tolerant parse facts, iterative validation graph, and bounded report shape while avoiding `ThreadLocal` session state and JavaScript rule execution.
- [../docs/research/spike-profile-expression-ir.md](../docs/research/spike-profile-expression-ir.md) — rule-expression risk retirement for generated built-in profile coverage.
- [../docs/research/spike-pdf-stream-resource-limits.md](../docs/research/spike-pdf-stream-resource-limits.md) — stream decode caps and scan caps for hostile PDFs.
- [../docs/research/spike-mrr-compatibility.md](../docs/research/spike-mrr-compatibility.md) — XML/MRR compatibility naming and report-surface decisions.
