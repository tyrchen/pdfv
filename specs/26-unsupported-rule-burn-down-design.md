# 26-unsupported-rule-burn-down-design: Unsupported Rule Burn-Down

Status: draft v1 · Owner: pdfv · Last updated: 2026-05-17 · Depends on: [16-profile-catalog-rule-parity-design.md](./16-profile-catalog-rule-parity-design.md), [17-validation-model-parity-design.md](./17-validation-model-parity-design.md), [22-resource-font-color-semantics-design.md](./22-resource-font-color-semantics-design.md), [18-xmp-metadata-flavour-design.md](./18-xmp-metadata-flavour-design.md), [23-structure-accessibility-design.md](./23-structure-accessibility-design.md), [24-parity-metrics-verification-plan.md](./24-parity-metrics-verification-plan.md), [25-verapdf-grade-validation-prd.md](./25-verapdf-grade-validation-prd.md)

## 1. Purpose

This spec turns unsupported official rules into an implementation backlog that can be reduced mechanically. It owns classification, prioritization, acceptance gates, and reporting for missing expression features, missing derived properties, missing semantic families, and expected out-of-scope rules.

The current review identifies the main blockers: 1,095 missing-property rules, 574 unsupported-expression rules, and 42 missing-semantic-family rules ([../docs/reviews/verapdf-feature-parity-gaps-review.md](../docs/reviews/verapdf-feature-parity-gaps-review.md)). The implementation must not treat these as generic "unsupported" noise; each cluster needs an owner, design target, fixtures, and a measurable exit condition.

## 2. Inputs and Outputs

Inputs:

- `target/parity/unsupported-rules.json`
- `target/parity/profile-coverage.json`
- `target/parity/model-schema.json`
- vendored veraPDF profile XML and source pins
- release corpus divergences from [27-oracle-corpus-verification-plan.md](./27-oracle-corpus-verification-plan.md)

Outputs:

- `target/parity/unsupported-rule-clusters.json`
- `target/parity/rule-burn-down.json`
- milestone review snapshots under `docs/reviews/`
- generated issue/backlog rows when automation is added

All new automation must be exposed through Makefile targets, not standalone scripts:

```text
make parity-unsupported-clusters
make parity-burn-down
```

## 3. Classification Model

Every unsupported rule has exactly one primary reason and zero or more secondary tags.

```rust
pub enum UnsupportedPrimaryReason {
    UnsupportedExpression,
    MissingProperty,
    MissingLink,
    MissingObjectType,
    MissingSemanticFamily,
    OutOfScope,
    VeraPdfAmbiguity,
}

pub struct UnsupportedRuleCluster {
    pub primary_reason: UnsupportedPrimaryReason,
    pub profile_family: ProfileFamily,
    pub object_type: ObjectTypeName,
    pub property: Option<PropertyName>,
    pub semantic_family: Option<SemanticFamilyName>,
    pub rules: Vec<RuleId>,
    pub release_blocking: bool,
    pub owner_spec: SpecReference,
}
```

Primary-reason rules:

- `UnsupportedExpression`: expression could not lower before model binding.
- `MissingProperty`: object type exists, but at least one referenced property is not implemented.
- `MissingLink`: source and target families exist, but traversal link is absent.
- `MissingObjectType`: profile object type is unknown to the registry.
- `MissingSemanticFamily`: object/property names exist only after a semantic subsystem is built, such as accessibility chunks or XMP RDF facts.
- `OutOfScope`: explicitly excluded by [25-verapdf-grade-validation-prd.md](./25-verapdf-grade-validation-prd.md#5-non-goals) or a key decision.
- `VeraPdfAmbiguity`: upstream behavior depends on undocumented, version-specific, or corpus-specific behavior and has a review note.

## 4. Priority Order

Burn-down order is binding for readiness phases:

1. Unsupported expressions that block many PDF/A rules.
2. Derived properties on already-registered families: `object`, `document`, `metadata`, `annotation`, `image`, `outputIntent`, `font`, `colorSpace`.
3. Font, CMap, font descriptor, and font-program properties.
4. Effective resource, color, image, XObject, ExtGState, ICC, pattern, shading, and function properties.
5. XMP RDF schema/property/array/qualifier facts.
6. Accessibility semantic families: structure elements, chunks, tables, lists, headings, links, annotations, artifacts, repeated characters, optional-content visibility.
7. Product-surface feature extraction gaps after validation decisions are stable.

This order follows the feature-parity review and prevents low-value feature work from hiding core validation gaps.

## 5. Workstream Contracts

### 5.1 Expression Parity

Owner specs: [12-profile-rule-ir-design.md](./12-profile-rule-ir-design.md), [16-profile-catalog-rule-parity-design.md](./16-profile-catalog-rule-parity-design.md).

Required closure:

- Parse official comparison edge cases currently reported as `expected expression`.
- Parse parenthesized compound booleans currently reported as `trailing expression input`.
- Support bounded arithmetic, modulo, flag-mask checks, ternary expressions, collection predicates, and regex checks used by in-scope profiles.
- Every newly supported grammar feature gets unit tests and profile-regression rows.

Exit metric: unsupported-expression count falls below 1% of imported in-scope rules, or every remaining expression has an `OutOfScope` or `VeraPdfAmbiguity` note.

### 5.2 Derived Property Backlog

Owner specs: [17-validation-model-parity-design.md](./17-validation-model-parity-design.md), [22-resource-font-color-semantics-design.md](./22-resource-font-color-semantics-design.md), [18-xmp-metadata-flavour-design.md](./18-xmp-metadata-flavour-design.md), [23-structure-accessibility-design.md](./23-structure-accessibility-design.md).

Required closure:

- Add typed property resolvers instead of expanding generic dictionary wrappers indefinitely.
- For each property, record source evidence: direct COS key, inherited key, parsed stream metadata, derived semantic summary, or oracle-only expected drift.
- Missing data in malformed PDFs becomes a rule-facing value or structured fact, not a panic.

Exit metric: missing-property count falls below 3% of imported in-scope rules, and no PDF/A profile has fewer than 95% bound rules unless the remainder is explicitly out-of-scope.

### 5.3 Semantic Family Closure

Owner specs: [22-resource-font-color-semantics-design.md](./22-resource-font-color-semantics-design.md), [23-structure-accessibility-design.md](./23-structure-accessibility-design.md).

Required closure:

- Implement resource-aware semantics before color/image/XObject validation rules that depend on active resources.
- Implement accessibility chunk/annotation/table/list/link/artifact families before claiming PDF/UA-grade readiness.
- Each semantic family has positive, negative, malformed, and oracle corpus fixtures.

Exit metric: missing-semantic-family count is zero for in-scope PDF/A and PDF/UA readiness, except explicitly out-of-scope trust/signature or rendering-only items.

## 6. Acceptance Gates

A rule cluster is closed only when all are true:

1. The relevant model or expression implementation exists.
2. The unsupported-rule count for that cluster decreases in `target/parity/rule-burn-down.json`.
3. At least one generated or checked-in fixture exercises the new behavior.
4. If the cluster affected live corpus divergence, at least one oracle row moves from mismatch to match or from untriaged to expected drift.
5. The implementation returns `Incomplete` rather than `Valid` when it hits a remaining required unsupported rule.

## 7. Reporting

Burn-down reports must include:

- current count and previous-snapshot count per cluster
- largest 20 clusters by rule count
- largest 20 clusters by release-corpus mismatch count
- owner spec and expected implementation phase
- release-blocking flag
- examples: profile, rule id, object type, property, expression fragment, and source XML path

The report must be deterministic and safe to commit at milestone boundaries.

## 8. AGENTS.md Binding

- Automation: new reports are Makefile targets.
- Error Handling: malformed profile XML or malformed unsupported-rule records produce structured errors.
- Safety & Security: report generation never reads arbitrary paths except configured corpus/profile roots; paths are canonicalized.
- Type Design & API: reason, cluster, rule id, property, and spec reference fields are newtyped.
- Testing: cluster grouping has unit tests and snapshot tests; coverage decreases require a review note.
- Documentation: every release-blocking cluster points to a spec section or review note.

## 9. Cross-references

- ← Depends on: [16-profile-catalog-rule-parity-design.md](./16-profile-catalog-rule-parity-design.md), [17-validation-model-parity-design.md](./17-validation-model-parity-design.md), [18-xmp-metadata-flavour-design.md](./18-xmp-metadata-flavour-design.md), [22-resource-font-color-semantics-design.md](./22-resource-font-color-semantics-design.md), [23-structure-accessibility-design.md](./23-structure-accessibility-design.md), [24-parity-metrics-verification-plan.md](./24-parity-metrics-verification-plan.md), [25-verapdf-grade-validation-prd.md](./25-verapdf-grade-validation-prd.md)
- → Consumed by: [27-oracle-corpus-verification-plan.md](./27-oracle-corpus-verification-plan.md), [28-verapdf-grade-validation-impl-plan.md](./28-verapdf-grade-validation-impl-plan.md)
- ↔ Related review: [../docs/reviews/verapdf-feature-parity-gaps-review.md](../docs/reviews/verapdf-feature-parity-gaps-review.md)
