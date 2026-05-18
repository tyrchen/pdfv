# M9 G6 PDF/UA Accessibility Reconstruction Snapshot

Status: Done | Owner: pdfv | Date: 2026-05-18

Vendor pin: `veraPDF-library` @ `acfcc419a5df444e3e8b2a18266d01e249299957`.

## Evidence

```bash
make parity-model-schema
make parity-corpus
make parity-profile-report
PDFV_PARITY_BASELINE_DIR=docs/reviews/m9-g5-xmp-rdf-metadata-validation make parity-burn-down
```

The JSON artifacts are generated under `target/parity/` and copied into
[`m9-g6-pdf-ua-accessibility-reconstruction/`](./m9-g6-pdf-ua-accessibility-reconstruction/)
for this milestone:

- `profile-coverage.json`
- `model-schema.json`
- `unsupported-rules.json`
- `corpus-agreement.json`
- `unsupported-rule-clusters.json`
- `rule-burn-down.json`

## G6 Summary

| Metric | Count |
| --- | ---: |
| Remaining unsupported rules | 0 |
| Remaining unsupported clusters | 0 |
| Release-blocking unsupported rules | 0 |
| Release-blocking unsupported clusters | 0 |
| G6 accessibility clusters | 0 |
| G6 release-blocking clusters | 0 |

G6 removed every remaining PDF/UA and WTPDF accessibility unsupported-rule
cluster. The implementation now binds structure-root child-type facts,
structure-element tag/language/heading/table/list/note/signature properties,
annotation target facts, and previously missing table/list semantic guard
properties. The accessibility graph remains bounded, lazy, and report-safe:
raw page text and alternate text are not dumped into feature reports.

## Bound Coverage

All PDF/A, PDF/UA, and WTPDF profile rules are lowered and bound after the G6
snapshot. PDF/UA readiness thresholds are fully met by rule binding:

| Flavour | Bound / total |
| --- | ---: |
| pdfua-1 | 106 / 106 |
| pdfua-2-iso32005 | 1,727 / 1,727 |
| wtpdf-1-0-accessibility | 1,723 / 1,723 |
| wtpdf-1-0-reuse | 1,710 / 1,710 |

PDF/A coverage remains fully bound from G5.

## Corpus

The Java-free parity corpus remains stable: 10 / 10 rows match, with 0 expected
drift and 0 unexpected drift. The generated accessibility row covers tagged
structure, role-map normalization, marked-content associations, image-alt,
artifact, list, link, and annotation semantic families.

`corpus-agreement.json` records `liveOracleEnabled: false`; live veraPDF oracle
execution remains part of G7 release-oracle automation. This G6 snapshot is the
bounded Java-free evidence for PDF/UA accessibility reconstruction.

## Remaining Readiness Work

No release-blocking unsupported-rule cluster remains after G6. The remaining
readiness work moves to G7 live oracle execution and drift triage, then G8
release documentation and hardening.
