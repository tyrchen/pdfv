# M9 G7 Release Oracle and Drift Triage Snapshot

Status: Done | Owner: pdfv | Date: 2026-05-18

Vendor pin: `veraPDF-library` @ `acfcc419a5df444e3e8b2a18266d01e249299957`.
Live oracle binary: `veraPDF 1.31.65`, downloaded from the veraPDF development
1.31 software directory on 2026-05-18. The listed installer was
`verapdf-greenfield-1.31.65-installer.zip`, dated 2026-05-11.

## Evidence

```bash
make parity-model-schema
make parity-corpus
make parity-profile-report
PDFV_PARITY_BASELINE_DIR=docs/reviews/m9-g6-pdf-ua-accessibility-reconstruction make parity-burn-down
PDFV_VERAPDF_BIN=target/tools/verapdf-runtime/verapdf \
  PDFV_ORACLE_CORPUS_MANIFEST=tests/oracle-t2-public-conformance.yml \
  PDFV_ORACLE_CORPUS_ROOT=. \
  make oracle-corpus
make oracle-corpus-summary
```

The JSON artifacts are generated under `target/parity/` and copied into
[`m9-g7-release-oracle-drift-triage/`](./m9-g7-release-oracle-drift-triage/)
for this milestone:

- `profile-coverage.json`
- `model-schema.json`
- `unsupported-rules.json`
- `corpus-agreement.json`
- `unsupported-rule-clusters.json`
- `rule-burn-down.json`
- `oracle-corpus.json`
- `oracle-summary.json`
- `oracle-drift.json`

## G7 Summary

| Metric | Count |
| --- | ---: |
| Live T2 rows | 3 |
| Live T2 matches | 0 |
| Live T2 classified mismatches | 3 |
| Live T2 unexpected drift | 0 |
| Live T2 false-compliant rows | 0 |
| Live T2 timeout rows | 0 |
| Live T3 committed rows | 0 |

All live T2 mismatches are classified as `expectedDriftUnsupportedRule`: veraPDF
returns `Valid` or `Invalid`, while pdfv returns `Incomplete` with unsupported
rule evidence. This preserves the G7 safety invariant that a live mismatch must
not become a false-compliant `Valid` result.

## Corpus Manifests

`tests/oracle-t2-public-conformance.yml` is the checked-in T2 manifest. It uses
redistributable vendored veraPDF fixtures and records profile policy, expected
semantic families, producer, size bucket, encryption class, tagging class, and
license class for each row.

`tests/oracle-t3-real-world-template.yml` is the checked-in T3 manifest template.
It intentionally has no committed private rows. Real-world T3 rows are supplied
through `PDFV_ORACLE_CORPUS_MANIFEST` and `PDFV_ORACLE_CORPUS_ROOT` during a
release-lab run so private document paths and provenance do not enter the repo.

## Drift Triage

| Row | veraPDF | pdfv | Classification |
| --- | --- | --- | --- |
| `t2-verapdf-cli-pass-a-pdfa-1b` | `valid` | `incomplete` | `expectedDriftUnsupportedRule` |
| `t2-verapdf-parser-valid-document-pdfa-1b` | `valid` | `incomplete` | `expectedDriftUnsupportedRule` |
| `t2-verapdf-feature-pages-pdfa-1b` | `invalid` | `incomplete` | `expectedDriftUnsupportedRule` |

The oracle runner enforces canonicalized corpus roots, argv-based process
execution, per-file timeout, bounded process output, and redacted relative paths
in reports. `oracle-summary.json` has 0 unexpected drift and 0 false-compliant
rows for the committed T2 run. The T3 release gate is implemented by the runner
and applies when a private T3 manifest is supplied.

## Remaining Readiness Work

No G7 automation gap remains. G8 can now run the standard repository gates plus
the oracle gates on a release candidate, then publish user-facing readiness
documentation with the supported profiles, corpus evidence, known expected
drift, and out-of-scope surfaces.
