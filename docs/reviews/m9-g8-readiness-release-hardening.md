# M9 G8 Readiness Release Hardening Snapshot

Status: Evidence Packaged, Release Claim Withheld | Owner: pdfv | Date:
2026-05-19

Vendor pin: `veraPDF-library` @
`acfcc419a5df444e3e8b2a18266d01e249299957`.
Live oracle binary: `veraPDF 1.31.65`, built 2026-05-11.

## Evidence Commands

```bash
PDFV_PARITY_BASELINE_DIR=docs/reviews/m9-g7-release-oracle-drift-triage \
PDFV_VERAPDF_BIN=target/tools/verapdf-runtime/verapdf \
  PDFV_ORACLE_CORPUS_MANIFEST=tests/oracle-t2-public-conformance.yml \
  PDFV_ORACLE_CORPUS_ROOT=. \
  make readiness-gates
```

The JSON artifacts are generated under `target/parity/` and copied into
[`m9-g8-readiness-release-hardening/`](./m9-g8-readiness-release-hardening/):

- `profile-coverage.json`
- `model-schema.json`
- `unsupported-rules.json`
- `corpus-agreement.json`
- `unsupported-rule-clusters.json`
- `rule-burn-down.json`
- `oracle-corpus.json`
- `oracle-summary.json`
- `oracle-drift.json`

## Release-Candidate Summary

| Metric | Count |
| --- | ---: |
| Imported profiles | 15 |
| Imported official rules | 6,749 |
| Lowered rules | 6,749 |
| Bound rules | 6,749 |
| Unsupported rules | 0 |
| Release-blocking unsupported clusters | 0 |
| Java-free parity corpus rows | 13 |
| Java-free parity corpus matches | 13 |
| Java-free parity corpus unexpected drift | 0 |
| Live T2 public oracle rows | 3 |
| Live T2 public oracle unexpected drift | 0 |
| Live T2 public oracle false-compliant rows | 0 |
| Live T3 release-lab rows | 0 |

## Supported Profiles In This Snapshot

The profile coverage artifact binds all imported rules for:

- PDF/A: `pdfa-1a`, `pdfa-1b`, `pdfa-2a`, `pdfa-2b`, `pdfa-2u`,
  `pdfa-3a`, `pdfa-3b`, `pdfa-3u`, `pdfa-4`, `pdfa-4e`, `pdfa-4f`;
- PDF/UA: `pdfua-1`, `pdfua-2-iso32005`;
- tracked but not first-claim gated: `wtpdf-1-0-accessibility`,
  `wtpdf-1-0-reuse`.

## Oracle Gate Status

The checked-in T2 public conformance manifest has 3 rows. Every row is
classified and none is false-compliant. The live T2 rows all classify as
`expectedDriftUnsupportedRule`, where veraPDF returns `Valid` or `Invalid` and
pdfv returns `Incomplete`.

The private T3 real-world release corpus has not been supplied in this
repository. Because the T3 gate has 0 release-lab rows, this snapshot does not
support a public claim that pdfv is veraPDF-grade for arbitrary real-world
PDF/A/PDF/UA collections.

## Release Gate Verdict

| Gate | Status | Evidence |
| --- | --- | --- |
| Standard repository gates | Passed | `make standard-gates`; `cargo deny check` emitted only the documented unmatched-license and duplicate `wit-bindgen` warnings |
| Parity profile/model/corpus snapshots | Passed | `profile-coverage.json`, `model-schema.json`, `corpus-agreement.json` |
| Unsupported-rule release blockers | Passed | `unsupported-rule-clusters.json` reports 0 release-blocking clusters |
| T2 public oracle gate | Passed | `oracle-summary.json` reports 0 unexpected drift and 0 false-compliant rows |
| T3 real-world oracle gate | Blocked | no private release-lab manifest was supplied; 0 T3 rows |
| Public docs and release notes | Passed | `docs/verapdf-readiness.md` and `docs/release-notes/m9-g8-verapdf-readiness.md` |

The correct public posture is therefore: profile/rule coverage and committed
oracle infrastructure are release-hardened, but the first veraPDF-grade
readiness claim remains blocked on the T3 release-lab evidence required by
`specs/27-oracle-corpus-verification-plan.md`.
