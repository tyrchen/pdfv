# M9 G0 Unsupported-Rule Baseline Snapshot

Status: Done · Owner: pdfv · Date: 2026-05-17

Vendor pin: `veraPDF-library` @ `acfcc419a5df444e3e8b2a18266d01e249299957`.

pdfv pin at snapshot generation: `b9785f6c6a56c2e1c9a9b22e95e8d7510e60fc8c`.

## Evidence

```bash
make parity-profile-report
make parity-model-schema
make parity-corpus
make parity-burn-down
```

The JSON artifacts are generated under `target/parity/` and copied into
[`m9-g0-unsupported-rule-baseline/`](./m9-g0-unsupported-rule-baseline/) for this milestone:

- `profile-coverage.json`
- `model-schema.json`
- `unsupported-rules.json`
- `corpus-agreement.json`
- `unsupported-rule-clusters.json`
- `rule-burn-down.json`

## Baseline Summary

| Metric | Count |
| --- | ---: |
| Unsupported rules | 1,711 |
| Unsupported clusters | 232 |
| Release-blocking rules | 1,401 |
| Release-blocking clusters | 187 |

Release-blocking cluster count by expected readiness phase:

| Phase | Clusters |
| --- | ---: |
| G1 | 20 |
| G2 | 77 |
| G3 | 21 |
| G4 | 32 |
| G5 | 21 |
| G6 | 16 |

Largest current clusters:

| Primary reason | Family | Object | Property/link/semantic | Rules | Phase | Release blocking |
| --- | --- | --- | --- | ---: | --- | --- |
| unsupportedExpression | wtpdf | structureElement | - | 182 | G1 | false |
| unsupportedExpression | pdfua | structureElement | - | 106 | G1 | true |
| unsupportedExpression | pdfa | font | - | 73 | G1 | true |
| unsupportedExpression | pdfa | object | - | 58 | G1 | true |
| unsupportedExpression | pdfa | annotation | - | 44 | G1 | true |
| missingProperty | pdfa | object | internalRepresentation | 30 | G2 | true |
| missingProperty | pdfa | image | hasColorSpace | 27 | G4 | true |
| missingProperty | pdfa | font | isSymbolic | 25 | G3 | true |
| missingProperty | pdfa | colorSpace | gOutputCS | 24 | G4 | true |
| missingProperty | pdfa | font | renderingMode | 24 | G3 | true |

## Release-Blocking Classification

Clusters are release-blocking when they affect in-scope PDF/A or PDF/UA profile families. WTPDF clusters are tracked for visibility but do not block the first PDF/A/PDF/UA readiness claim.

The baseline confirms the readiness plan order: expression closure is the largest first blocker, followed by derived properties, font/CMap semantics, resource/color/image semantics, XMP RDF metadata facts, and accessibility reconstruction.
