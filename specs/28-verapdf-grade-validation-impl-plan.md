# 28-verapdf-grade-validation-impl-plan: Implementation Plan for veraPDF-Grade Readiness

Status: draft v1 · Owner: pdfv · Last updated: 2026-05-17 · Depends on: [25-verapdf-grade-validation-prd.md](./25-verapdf-grade-validation-prd.md), [26-unsupported-rule-burn-down-design.md](./26-unsupported-rule-burn-down-design.md), [27-oracle-corpus-verification-plan.md](./27-oracle-corpus-verification-plan.md)

## 1. Readiness Assessment

Ready:

- The base implementation plan through M8 defines parser, profile, model, content, resource, XMP, accessibility, parity metrics, feature, and report surfaces.
- The parity harness already emits profile coverage, model schema, unsupported-rule, and Java-free corpus reports.
- The feature-parity review identifies concrete blocker classes and current counts.

Not ready:

- PDF/A profile bound-rule counts are far below a release-grade bar.
- Unsupported expressions and missing derived properties are still broad enough to block decision-grade validation.
- Live veraPDF oracle comparison is opt-in and not yet a release gate.
- Existing corpus rows are useful smoke tests, not real-world readiness evidence.

This plan starts after the M5-M8 foundation in [91-impl-plan.md](./91-impl-plan.md) is present. It can overlap with late M6-M8 only when write scopes are disjoint.

## 2. Why This Plan Is Separate From 91

[91-impl-plan.md](./91-impl-plan.md) builds the engine. This plan hardens the engine into a release claim. The dependency order is different:

- Unsupported-rule burn-down must be metrics-driven, not component-driven.
- Oracle agreement work must land before any public "veraPDF-grade" claim.
- Product-surface polish is lower priority than avoiding false-compliant validation decisions.

## 3. Estimated Effort

For one focused developer after M8 foundation exists: 16-32 developer-weeks.

The critical path is expression coverage, derived properties, font/color/resource semantics, XMP RDF facts, accessibility reconstruction, and oracle triage. Parallelism helps with fixture authoring and oracle harness work, but semantic implementation remains mostly serial because rule clusters depend on shared model contracts.

## 4. Phase G0 - Baseline Snapshot and Backlog

| # | Task | Spec | Effort |
| --- | --- | --- | --- |
| G0.1 | Regenerate `profile-coverage.json`, `model-schema.json`, `unsupported-rules.json`, and `corpus-agreement.json`; copy a milestone snapshot to `docs/reviews/`. | 24, 25 | 0.5 day |
| G0.2 | Implement `make parity-unsupported-clusters` and `make parity-burn-down`. | 26, 61 | 1-2 days |
| G0.3 | Classify current unsupported rules by primary reason, profile family, object type, property, semantic family, and owner spec. | 26 | 2-4 days |
| G0.4 | Mark release-blocking clusters for PDF/A and PDF/UA readiness. | 25, 26 | 1 day |

Exit criteria: unsupported-rule clusters are deterministic, every rule has one primary reason, and the top release-blocking clusters are visible in a review snapshot.

## 5. Phase G1 - Expression Coverage Closure

| # | Task | Spec | Effort |
| --- | --- | --- | --- |
| G1.1 | Extend the bounded expression grammar for official comparison, arithmetic, modulo, flag-mask, ternary, and collection predicate fragments. | 12, 16, 26 | 1-2 weeks |
| G1.2 | Add profile-regression tests for every newly accepted expression shape. | 16, 72 | 3-5 days |
| G1.3 | Reclassify remaining unsupported expressions as release-blocking, out-of-scope, or veraPDF ambiguity. | 26 | 2-4 days |

Exit criteria: unsupported-expression count is below 1% of imported in-scope rules or every remaining expression has a review-backed classification; no false-compliant outcome is possible from unsupported expressions.

## 6. Phase G2 - Derived Property Closure

| # | Task | Spec | Effort |
| --- | --- | --- | --- |
| G2.1 | Implement typed derived-property resolvers for `object`, `document`, `metadata`, `annotation`, `image`, and `outputIntent`. | 17, 18, 22, 26 | 2-4 weeks |
| G2.2 | Add source-evidence metadata for each derived property: direct COS, inherited COS, decoded stream, XMP RDF, semantic graph, or expected drift. | 17, 24, 26 | 1 week |
| G2.3 | Add generated fixtures for the top missing-property clusters. | 72, 26 | 1-2 weeks |

Exit criteria: missing-property count falls materially from baseline; no in-scope PDF/A profile remains below 60% bound rules unless blocked by font/resource semantics scheduled in G3/G4.

## 7. Phase G3 - Font and CMap Decision Semantics

| # | Task | Spec | Effort |
| --- | --- | --- | --- |
| G3.1 | Implement font subtype, descriptor, embedding, encoding, ToUnicode, widths, symbolic/nonsymbolic, and CIDSystemInfo derived facts. | 22, 26 | 3-6 weeks |
| G3.2 | Implement CMap and embedded font-file summaries sufficient for official PDF/A rules. | 22, 70 | 2-4 weeks |
| G3.3 | Add oracle rows for font-heavy PDF/A valid and invalid cases. | 27, 72 | 1-2 weeks |

Exit criteria: `font` and `cMap` unsupported clusters stop dominating PDF/A profiles; PDF/A bound-rule count improves toward the 95% readiness bar; oracle font rows have no unexpected drift.

## 8. Phase G4 - Effective Resource, Color, Image, and XObject Semantics

| # | Task | Spec | Effort |
| --- | --- | --- | --- |
| G4.1 | Complete effective resource resolution across pages, forms, patterns, and Type 3 charprocs. | 21, 22, 26 | 2-4 weeks |
| G4.2 | Implement color/output-intent/ICC/ExtGState/image/XObject properties required by remaining PDF/A clusters. | 22, 26 | 3-6 weeks |
| G4.3 | Add fixtures and oracle rows for output intents, ICC profiles, DeviceN/Separation, transparency, soft masks, images, and nested XObjects. | 27, 72 | 2-4 weeks |

Exit criteria: color/resource/image/XObject clusters are no longer release-blocking for PDF/A; resource-related oracle rows have no unexpected drift.

## 9. Phase G5 - XMP RDF Metadata Validation

| # | Task | Spec | Effort |
| --- | --- | --- | --- |
| G5.1 | Expand XMP parsing from identification claims to bounded RDF schema/property/array/qualifier facts. | 18, 70, 26 | 3-6 weeks |
| G5.2 | Bind official metadata rules to XMP fact properties and classify remaining metadata gaps. | 16, 17, 18, 26 | 1-2 weeks |
| G5.3 | Add fixtures for malformed, duplicate, extension-schema, namespace-prefix, encrypted-metadata, and repair-relevant metadata cases. | 18, 27, 72 | 1-2 weeks |

Exit criteria: `metadata` unsupported clusters are below the readiness threshold; XMP oracle rows have no unexpected drift; reports never dump full XMP packets.

## 10. Phase G6 - PDF/UA Accessibility Reconstruction

| # | Task | Spec | Effort |
| --- | --- | --- | --- |
| G6.1 | Deepen marked-content to structure-element association using resolved resources and parent-tree data. | 21, 22, 23, 26 | 2-4 weeks |
| G6.2 | Implement text/image chunk, annotation/link, table/list/heading, artifact, repeated-character, and optional-content visibility facts. | 23, 26 | 4-8 weeks |
| G6.3 | Add PDF/UA oracle rows for tagged/untagged, role-map, table/list/link/image-alt/artifact, and malformed MCID cases. | 27, 72 | 2-4 weeks |

Exit criteria: PDF/UA-1 and PDF/UA-2 reach readiness thresholds; missing-semantic-family count is zero for in-scope PDF/UA; accessibility oracle rows have no unexpected drift.

## 11. Phase G7 - Release Oracle and Drift Triage

| # | Task | Spec | Effort |
| --- | --- | --- | --- |
| G7.1 | Implement `make oracle-corpus` and `make oracle-corpus-summary`. | 27, 61 | 1-2 weeks |
| G7.2 | Build T2 public conformance and T3 real-world corpus manifests with stratification fields. | 27, 72 | 1-3 weeks |
| G7.3 | Run live oracle comparison, classify every mismatch, and feed release-blocking clusters back into G1-G6 as needed. | 26, 27 | 2-6 weeks |
| G7.4 | Publish milestone oracle snapshot under `docs/reviews/`. | 25, 27 | 1-2 days |

Exit criteria: T0/T1/T4 pass in normal CI; T2 has zero unexpected drift and zero false-compliant rows; T3 reaches at least 95% outcome-class match with 100% mismatch classification.

## 12. Phase G8 - Readiness Release Hardening

| # | Task | Spec | Effort |
| --- | --- | --- | --- |
| G8.1 | Run all standard quality gates plus oracle gates on the release candidate. | 25, 27 | 1-2 days |
| G8.2 | Update user/developer docs with supported profiles, readiness scope, corpus evidence, known drift, and out-of-scope surfaces. | 25, 50, 72 | 2-4 days |
| G8.3 | Add release notes and a permanent key decision for the first veraPDF-grade claim. | 25, 99 | 1 day |

Exit criteria: public docs can state exactly which PDF/A/PDF/UA workflows are readiness-gated, what evidence backs the claim, and what remains unsupported.

## 13. Cross-references

- ← Depends on: [25-verapdf-grade-validation-prd.md](./25-verapdf-grade-validation-prd.md), [26-unsupported-rule-burn-down-design.md](./26-unsupported-rule-burn-down-design.md), [27-oracle-corpus-verification-plan.md](./27-oracle-corpus-verification-plan.md)
- → Consumed by: [90-roadmap.md](./90-roadmap.md), [91-impl-plan.md](./91-impl-plan.md), [99-key-decisions.md](./99-key-decisions.md)
- ↔ Related review: [../docs/reviews/verapdf-feature-parity-gaps-review.md](../docs/reviews/verapdf-feature-parity-gaps-review.md)
