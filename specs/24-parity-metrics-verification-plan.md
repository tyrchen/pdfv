# 24-parity-metrics-verification-plan: Parity Metrics and Verification

Status: draft · Owner: pdfv · Depends on: [16-profile-catalog-rule-parity-design.md](./16-profile-catalog-rule-parity-design.md), [17-validation-model-parity-design.md](./17-validation-model-parity-design.md), [21-content-stream-operator-model-design.md](./21-content-stream-operator-model-design.md), [22-resource-font-color-semantics-design.md](./22-resource-font-color-semantics-design.md), [23-structure-accessibility-design.md](./23-structure-accessibility-design.md)

## 1. Purpose

This verification plan makes feature parity measurable. It turns the drift review's recommended metrics into CI-visible artifacts so the project does not confuse imported XML volume with executable validator parity.

## 2. Metrics

| Metric | Definition | Gate |
| --- | --- | --- |
| Imported rules by flavour | Count of rules loaded from each vendored profile XML. | Must be stable for fixed vendor pins. |
| Executable rules by flavour | Rules whose expressions lower to bounded `RuleExpr`. | Must not decrease without an explicit review note. |
| Bound rules by object/property | Executable rules whose object type, properties, and links are present in the model registry. | Must increase in model parity phases. |
| Unsupported rules by reason | Group by missing expression feature, missing object type, missing property, missing link, missing semantic family. | Every unsupported rule has exactly one primary reason. |
| Model family coverage | Implemented model families vs families listed in [17](./17-validation-model-parity-design.md), [21](./21-content-stream-operator-model-design.md), [22](./22-resource-font-color-semantics-design.md), and [23](./23-structure-accessibility-design.md). | Each parity phase publishes before/after counts. |
| Feature family coverage | Feature report families with semantic fixtures and golden output. | M8 requires all in-scope feature families to have fixtures. |
| Corpus agreement | `pdfv` vs veraPDF semantic outcome for curated fixtures: valid/invalid/incomplete/parse-failed/encrypted. | Gated only for checked-in/generated fixtures; external corpus tests are ignored/opt-in. |

## 3. Artifacts

Add Makefile-discoverable commands rather than ad hoc scripts:

```text
make parity-profile-report
make parity-model-schema
make parity-corpus
```

Expected outputs:

- `target/parity/profile-coverage.json`
- `target/parity/model-schema.json`
- `target/parity/unsupported-rules.json`
- `target/parity/corpus-agreement.json`

Docs snapshots may be committed under `docs/reviews/` only at milestone boundaries. Transient reports stay under `target/`.

## 4. Profile Coverage Schema

```json
{
  "vendorPins": {
    "veraPDF-library": "acfcc419a5df444e3e8b2a18266d01e249299957"
  },
  "profiles": [
    {
      "flavour": "pdfa-1b",
      "source": "PDFA-1B.xml",
      "totalRules": 0,
      "loweredRules": 0,
      "boundRules": 0,
      "unsupportedByReason": {
        "missingProperty": 0,
        "missingObjectType": 0,
        "unsupportedExpression": 0,
        "missingSemanticFamily": 0
      }
    }
  ]
}
```

The committed report examples must use nonzero real counts once the generator exposes them. Placeholder zero values are forbidden in milestone reports.

## 5. Corpus Agreement

Corpus rows are semantic, not byte-identical report comparisons:

| Field | Meaning |
| --- | --- |
| `fixture` | Local path or external corpus id. |
| `source` | Generated, checked-in, or external ignored corpus. |
| `veraPdfOutcome` | Upstream expected outcome or live oracle outcome when enabled. |
| `pdfvOutcome` | pdfv outcome. |
| `profile` | Selected flavour/profile. |
| `agreement` | `match`, `expected-drift`, or `unexpected-drift`. |
| `reason` | Required for every non-match. |

External live-oracle runs are opt-in because they require Java veraPDF and corpus licensing. Normal CI uses generated and checked-in fixtures only.

## 6. Invariants

- Coverage counters are complete even when assertion details are capped.
- Unsupported-rule reports are deterministic and sorted by profile, rule id, object type, then reason.
- A rule cannot be counted as bound unless its object type, every referenced property, every referenced link, and every required semantic family are registered.
- Metrics never require raw PDF bytes or secrets in output.
- Decreases in executable/bound coverage require an explicit review entry under `docs/reviews/` or `specs/93-improvements-review.md`.

## 7. AGENTS.md Binding

- Automation: all parity commands are Makefile targets.
- Error Handling: metrics generation returns structured errors; no panics on malformed profile XML or fixtures.
- Safety & Security: external corpus paths are canonicalized; live-oracle commands never interpolate user input through a shell.
- Testing: metric schema has unit tests and snapshot tests with deterministic ordering.
- Documentation: milestone parity reports cite vendor pins and the spec phase that produced them.

## 8. Cross-references

- ← Depends on: [16-profile-catalog-rule-parity-design.md](./16-profile-catalog-rule-parity-design.md), [17-validation-model-parity-design.md](./17-validation-model-parity-design.md), [21-content-stream-operator-model-design.md](./21-content-stream-operator-model-design.md), [22-resource-font-color-semantics-design.md](./22-resource-font-color-semantics-design.md), [23-structure-accessibility-design.md](./23-structure-accessibility-design.md)
- → Consumed by: [90-roadmap.md](./90-roadmap.md), [91-impl-plan.md](./91-impl-plan.md), [72-testing-strategy.md](./72-testing-strategy.md)
- ↔ Related review: [../docs/reviews/verapdf-pdfv-core-drift-review.md](../docs/reviews/verapdf-pdfv-core-drift-review.md)
