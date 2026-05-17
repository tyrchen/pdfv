# 27-oracle-corpus-verification-plan: Oracle Corpus Verification

Status: draft v1 · Owner: pdfv · Last updated: 2026-05-17 · Depends on: [24-parity-metrics-verification-plan.md](./24-parity-metrics-verification-plan.md), [25-verapdf-grade-validation-prd.md](./25-verapdf-grade-validation-prd.md), [26-unsupported-rule-burn-down-design.md](./26-unsupported-rule-burn-down-design.md)

## 1. Purpose

This verification plan defines the evidence required before pdfv can claim veraPDF-grade validation for arbitrary real-world PDF/A and PDF/UA workflows. It expands [24-parity-metrics-verification-plan.md](./24-parity-metrics-verification-plan.md) from generated metrics into a release oracle process with live veraPDF comparison, corpus stratification, drift triage, and acceptance gates.

Normal CI remains Java-free and deterministic. Release readiness adds explicit opt-in oracle runs against pinned veraPDF binaries and curated corpora.

## 2. Corpus Tiers

| Tier | Source | Runs in normal CI | Purpose |
| --- | --- | --- | --- |
| T0 Generated fixtures | In-process generated PDFs and malformed inputs | Yes | Pin individual parser/model/rule invariants. |
| T1 Checked-in fixtures | Small repository fixtures with clear provenance | Yes | Prevent regression without external dependencies. |
| T2 Public conformance fixtures | Redistributable PDF/A, PDF/UA, malformed, encrypted, tagged, and metadata fixtures | Optional in CI; required for release snapshots | Exercise known standard edge cases. |
| T3 Real-world corpus | User-provided or internal PDFs, not committed | No; release lab only | Measure arbitrary-document robustness and producer diversity. |
| T4 Adversarial corpus | Fuzz minimizations, decompression bombs, cyclic graphs, hostile XML, huge resources | Yes for minimized fixtures; optional for large private set | Preserve safety and resource-limit guarantees. |

Every corpus row has a manifest entry. Unknown provenance documents are allowed only in T3 and must not be committed.

## 3. Manifest Schema

```rust
pub struct OracleCorpusRow {
    pub id: CorpusRowId,
    pub path: CorpusPath,
    pub tier: CorpusTier,
    pub profile_policy: ProfilePolicy,
    pub expected_features: Vec<SemanticFamilyName>,
    pub producer: Option<BoundedText>,
    pub size_bucket: SizeBucket,
    pub encryption: EncryptionClass,
    pub tagging: TaggingClass,
    pub license: CorpusLicenseClass,
}
```

The manifest is YAML for humans and serializes to JSON in reports. Paths are relative to configured corpus roots and are canonicalized before use.

## 4. Oracle Execution

Add Makefile targets:

```text
make oracle-corpus
make oracle-corpus-summary
```

Runtime contract:

- Requires `PDFV_VERAPDF_BIN` or a documented default executable lookup.
- Requires `PDFV_ORACLE_CORPUS_MANIFEST`.
- Records veraPDF version, pdfv commit, vendor pins, OS, Java version, command-line profile selection, and report format.
- Runs veraPDF and pdfv with equivalent profile/flavour selection where possible.
- Compares outcome class first; assertion-level and report-field comparison are secondary diagnostics.
- Does not invoke shells with interpolated corpus paths.

## 5. Agreement Classification

Every row receives one classification:

| Classification | Meaning |
| --- | --- |
| `match` | pdfv and veraPDF agree on outcome class. |
| `expectedDriftUnsupportedRule` | pdfv returns `Incomplete` because a known unsupported rule cluster blocks a veraPDF `Valid` or `Invalid` decision. |
| `expectedDriftScope` | Difference is tied to an explicit non-goal or out-of-scope profile surface. |
| `expectedDriftLimit` | pdfv hits a configured resource limit and reports the limit clearly. |
| `veraPdfAmbiguity` | Upstream behavior is version-sensitive or ambiguous and has a review note. |
| `unexpectedDrift` | Any untriaged mismatch. Release blocker. |
| `pdfvBug` | Confirmed incorrect pdfv behavior. Release blocker until fixed or reclassified with a key decision. |

`Valid` from pdfv while veraPDF returns `Invalid` is always release-blocking until proven `veraPdfAmbiguity`; it cannot be hidden as expected drift.

## 6. Release Gates

For the first veraPDF-grade readiness claim:

- T0/T1/T4 normal CI: 100% pass, no unexpected drift, no panic/OOM.
- T2 public conformance: 100% of rows classified, 0 unexpected drift, 0 false-compliant rows.
- T3 real-world release corpus: at least 95% outcome-class match on in-scope profiles; 100% of mismatches classified; 0 false-compliant rows.
- Each in-scope profile family has enough T2/T3 rows to cover parser filters, encryption states, XMP states, font families, resource/color families, tagged/untagged states, annotations/actions/forms, and structure/accessibility families.
- Performance and resource-limit summaries are attached to the release snapshot.

Corpus size thresholds are release-specific and recorded in the milestone review. The first release-grade snapshot must not use fewer than 1,000 T3 documents unless [99-key-decisions.md](./99-key-decisions.md) records why a smaller corpus is statistically acceptable.

## 7. Oracle Report Schema

Expected outputs:

- `target/parity/oracle-corpus.json`
- `target/parity/oracle-summary.json`
- `target/parity/oracle-drift.json`

Summary fields:

- row counts by tier/profile/outcome/classification
- match percentage by profile family
- false-compliant count
- unexpected-drift count
- top unsupported clusters causing expected drift
- parse/resource-limit outcomes by size bucket
- slowest rows and timeout count
- veraPDF and pdfv version/pin block

Reports must not include raw PDF text, metadata packets, passwords, or absolute private paths.

## 8. Timeout and Resource Policy

Oracle runs are high-stakes verification, but they still treat PDFs as hostile:

- pdfv uses production resource limits unless a release note explicitly tests an expanded limit profile.
- veraPDF execution has per-file timeout, max output bytes, and process isolation.
- pdfv and veraPDF failures are recorded separately from validation outcomes.
- A timeout cannot be counted as a match.

## 9. AGENTS.md Binding

- Automation: oracle runs are Makefile targets with environment-variable configuration.
- Error Handling: process failures and malformed manifests produce structured oracle errors.
- Safety & Security: corpus paths are canonicalized; no shell interpolation; reports redact private paths and document content.
- Testing: manifest parser and classifier have unit tests; minimized T4 fixtures stay in normal CI.
- Performance: release summaries include runtime distribution and memory/resource-limit counts.
- Documentation: release snapshots cite corpus tier counts, exclusions, and known expected drift.

## 10. Cross-references

- ← Depends on: [24-parity-metrics-verification-plan.md](./24-parity-metrics-verification-plan.md), [25-verapdf-grade-validation-prd.md](./25-verapdf-grade-validation-prd.md), [26-unsupported-rule-burn-down-design.md](./26-unsupported-rule-burn-down-design.md)
- → Consumed by: [28-verapdf-grade-validation-impl-plan.md](./28-verapdf-grade-validation-impl-plan.md), [90-roadmap.md](./90-roadmap.md)
- ↔ Related research/review: [../docs/research/study-verapdf-e2e-tests.md](../docs/research/study-verapdf-e2e-tests.md), [../docs/reviews/verapdf-feature-parity-gaps-review.md](../docs/reviews/verapdf-feature-parity-gaps-review.md)
