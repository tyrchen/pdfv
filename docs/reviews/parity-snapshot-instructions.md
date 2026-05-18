# Parity Snapshot Instructions

Status: draft · Owner: pdfv · Last updated: 2026-05-17

Parity reports are transient by default and are generated under `target/parity/`:

```bash
make parity-profile-report
make parity-model-schema
make parity-corpus
make parity-unsupported-clusters
make parity-burn-down
PDFV_VERAPDF_BIN=target/tools/verapdf-runtime/verapdf \
  PDFV_ORACLE_CORPUS_MANIFEST=tests/oracle-t2-public-conformance.yml \
  PDFV_ORACLE_CORPUS_ROOT=. \
  make oracle-corpus
make oracle-corpus-summary
```

Milestone snapshots may be copied into `docs/reviews/` after the milestone gates pass. A snapshot must include the veraPDF vendor pin from the JSON report and the implementation phase that produced it.

Coverage decreases are review-gated. To compare against a prior snapshot, point `PDFV_PARITY_BASELINE_DIR` at a directory containing `profile-coverage.json` before running `make parity-profile-report`. If `loweredRules` or `boundRules` decreases for any profile, the target fails unless the decrease is documented by either:

- setting `PDFV_PARITY_REVIEW_NOTE` to an existing Markdown review note, or
- adding a `coverage decrease` entry to `docs/reviews/` or `specs/93-improvements-review.md`.

`make parity-unsupported-clusters` writes `target/parity/unsupported-rule-clusters.json`.
`make parity-burn-down` writes both `unsupported-rule-clusters.json` and `rule-burn-down.json`.
If `PDFV_PARITY_BASELINE_DIR` points to a previous snapshot directory that contains
`unsupported-rule-clusters.json`, the burn-down report includes previous counts and deltas.

`make parity-corpus` runs only generated and checked-in semantic rows and does not require Java. Live veraPDF corpus checks remain opt-in through `PDFV_VERAPDF_CORPUS_DIR` and `make test-conformance-verapdf`.

`make oracle-corpus` runs the release-oracle manifest named by
`PDFV_ORACLE_CORPUS_MANIFEST`. The manifest paths are resolved under
`PDFV_ORACLE_CORPUS_ROOT` when set; otherwise they are resolved under the
manifest's `corpusRoot`. The target writes `oracle-corpus.json`,
`oracle-summary.json`, and `oracle-drift.json` under `target/parity/`.
`make oracle-corpus-summary` rebuilds summary and drift artifacts from an
existing `oracle-corpus.json`.
