# 25-verapdf-grade-validation-prd: veraPDF-Grade Validation Readiness

Status: draft v1 · Owner: pdfv · Last updated: 2026-05-17 · Depends on: [16-profile-catalog-rule-parity-design.md](./16-profile-catalog-rule-parity-design.md), [17-validation-model-parity-design.md](./17-validation-model-parity-design.md), [22-resource-font-color-semantics-design.md](./22-resource-font-color-semantics-design.md), [18-xmp-metadata-flavour-design.md](./18-xmp-metadata-flavour-design.md), [23-structure-accessibility-design.md](./23-structure-accessibility-design.md), [24-parity-metrics-verification-plan.md](./24-parity-metrics-verification-plan.md)

## 1. Problem

pdfv has a strong parser, CLI, profile importer, validation model registry, and parity metric harness, but it is not yet ready to claim veraPDF-grade PDF/A or PDF/UA validation on arbitrary real-world PDFs.

The current parity review records 6,749 imported rules, 6,175 lowered rules, and 5,038 bound rules, but also 1,095 missing-property rules, 574 unsupported-expression rules, and 42 missing-semantic-family rules ([../docs/reviews/verapdf-feature-parity-gaps-review.md](../docs/reviews/verapdf-feature-parity-gaps-review.md)). More importantly, PDF/A profiles bind only 25-33 rules each, so aggregate rule coverage overstates production readiness for common archival validation workflows.

Without a release-grade readiness spec, the project can keep increasing counters while still missing the user-visible bar: reliable valid/invalid/incomplete agreement with veraPDF for real documents, clear unsupported-rule accounting, and no silent compliant results when required semantics are absent.

## 2. Vision

pdfv becomes a Rust-native validator that can be used in production PDF/A and PDF/UA workflows when teams need veraPDF-grade confidence with stronger memory-safety, bounded-resource, and integration properties. "veraPDF-grade" means decision-grade agreement and explainable drift against veraPDF's validation behavior for in-scope profiles, not byte-identical reports or an endorsement by veraPDF.

Example target workflow:

```bash
pdfv validate ./archive --recursive --flavour auto --format json --jobs 8 \
  --output ./reports/pdfv-validation.json
```

For every document, pdfv either reports the same semantic outcome class as veraPDF for the selected in-scope profile, or reports `Incomplete` with rule-level reasons precise enough to route implementation work.

## 3. Users

| User | Job |
| --- | --- |
| Archivist / records engineer | Validate large PDF/A collections before ingestion or migration. |
| Accessibility QA engineer | Validate tagged PDFs against PDF/UA and WTPDF-adjacent rules with actionable diagnostics. |
| CI/platform engineer | Run deterministic PDF validation in CI without JVM operational burden where pdfv's readiness gates are satisfied. |
| Library integrator | Embed validation and feature extraction in Rust services with bounded resource limits and structured reports. |
| pdfv maintainer | Prioritize implementation work by unsupported-rule clusters and oracle agreement evidence. |

## 4. Goals

| # | Goal | Measure |
| --- | --- | --- |
| G1 | Decision-grade profile coverage for PDF/A. | Each PDF/A profile has at least 95% bound official rules or every remaining unbound required rule is explicitly classified as out-of-scope for the readiness release. |
| G2 | Decision-grade profile coverage for PDF/UA. | PDF/UA-1 and PDF/UA-2 have at least 90% bound official rules plus no untriaged missing semantic family for structure, chunks, annotations, or metadata. |
| G3 | Real-world oracle agreement. | On the release oracle corpus, pdfv matches veraPDF outcome class for at least 95% of in-scope rows, with 100% of mismatches classified as expected drift, veraPDF ambiguity, parser limit, unsupported rule, or pdfv bug. |
| G4 | No false-compliant unsupported semantics. | If an in-scope required rule cannot execute, the affected profile report status is `Incomplete`, not `Valid`. |
| G5 | Arbitrary-input resilience. | Corpus, fuzz, and hostile-fixture gates show no panic, OOM, unchecked recursion, or unbounded decode path. |
| G6 | Operator usability. | CLI and library reports expose vendor pins, selected profiles, unsupported rules, oracle-drift evidence, and bounded feature facts needed to debug failures. |

## 5. Non-goals

- Byte-for-byte report compatibility with veraPDF XML, HTML, or text output.
- JavaScript engine embedding for profile rules.
- Pixel rendering, visual diffing, OCR, or assistive-technology simulation.
- Full cryptographic signature trust validation. Signature dictionary and permission facts are in scope; PKI trust-chain validation is not.
- Guaranteed compatibility with hostile PDFs beyond configured resource limits. Over-limit documents may produce `Incomplete` or parse-limit outcomes.
- Legal certification that pdfv is an official veraPDF replacement.

## 6. Readiness Definitions

| Term | Meaning |
| --- | --- |
| In-scope profile | PDF/A-1/2/3/4, PDF/UA-1, and PDF/UA-2 profiles imported from the vendored veraPDF profile XML set. WTPDF profiles are tracked but do not gate the first PDF/A/PDF/UA readiness claim. |
| Outcome class | One of `Valid`, `Invalid`, `Incomplete`, `Encrypted`, or `ParseFailed`. Fine-grained assertion differences are tracked separately. |
| Bound rule | A rule whose expression lowers successfully and whose object type, properties, links, and semantic family are registered and executable. |
| Decision-grade agreement | pdfv and veraPDF agree on outcome class for a corpus row, or pdfv returns `Incomplete` with a known unsupported-rule reason where veraPDF returns `Valid` or `Invalid`. |
| False compliant | pdfv returns `Valid` for a profile while any required in-scope rule is unsupported, skipped without accounting, or aborted due to semantic unavailability. False compliant results are release blockers. |

## 7. Success Metrics

The release readiness dashboard must include:

- Per-profile imported/lowered/bound/unsupported counts.
- Unsupported-rule burn-down by `primaryReason`, `objectType`, property name, and profile.
- Corpus outcome agreement by profile family, PDF producer, document feature family, file size bucket, encryption state, and tagged/untagged status.
- Parse completion and resource-limit outcomes by corpus.
- Fuzz coverage and crash count.
- Performance P50/P95/P99 for representative small, medium, and large PDFs.

## 8. Release Gate

The project may claim "veraPDF-grade for in-scope PDF/A/PDF/UA workflows" only when all of these are true:

1. `make parity-profile-report`, `make parity-model-schema`, and `make parity-corpus` pass and publish milestone snapshots under `docs/reviews/`.
2. [27-oracle-corpus-verification-plan.md](./27-oracle-corpus-verification-plan.md) release corpus gates pass.
3. [26-unsupported-rule-burn-down-design.md](./26-unsupported-rule-burn-down-design.md) reports no release-blocking unsupported-rule cluster.
4. Standard repository gates pass: `cargo build`, `cargo test`, `cargo +nightly fmt`, `cargo clippy -- -D warnings -W clippy::pedantic`, `cargo audit`, and `cargo deny check`.
5. The release notes list supported profiles, known expected drift, and out-of-scope surfaces.

## 9. AGENTS.md Binding

- Error Handling: in-scope unsupported semantics must become structured `Incomplete` reports, not panics or silent passes.
- Safety & Security: all readiness work preserves `#![forbid(unsafe_code)]`, hostile-input limits, checked arithmetic on external values, and no raw secret/text leakage in reports.
- Type Design & API: readiness metrics use newtyped profile ids, corpus ids, rule ids, object type names, and drift classifications.
- Testing: every release gate is Makefile-discoverable and deterministic by default; live veraPDF oracle runs remain explicit and reproducible.
- Documentation: any public readiness claim must cite the exact milestone snapshot, vendor pins, and known drift.

## 10. Cross-references

- ← Depends on: [16-profile-catalog-rule-parity-design.md](./16-profile-catalog-rule-parity-design.md), [17-validation-model-parity-design.md](./17-validation-model-parity-design.md), [18-xmp-metadata-flavour-design.md](./18-xmp-metadata-flavour-design.md), [22-resource-font-color-semantics-design.md](./22-resource-font-color-semantics-design.md), [23-structure-accessibility-design.md](./23-structure-accessibility-design.md), [24-parity-metrics-verification-plan.md](./24-parity-metrics-verification-plan.md)
- → Consumed by: [26-unsupported-rule-burn-down-design.md](./26-unsupported-rule-burn-down-design.md), [27-oracle-corpus-verification-plan.md](./27-oracle-corpus-verification-plan.md), [28-verapdf-grade-validation-impl-plan.md](./28-verapdf-grade-validation-impl-plan.md)
- ↔ Related review: [../docs/reviews/verapdf-feature-parity-gaps-review.md](../docs/reviews/verapdf-feature-parity-gaps-review.md)
