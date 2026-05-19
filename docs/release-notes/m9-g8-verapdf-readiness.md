# M9 G8 veraPDF Readiness Release Notes

Status: release-hardening evidence packaged; veraPDF-grade public claim
withheld pending T3 release-lab evidence.

## What Changed

- Added a G8 readiness snapshot with regenerated parity coverage, unsupported
  cluster, Java-free corpus, live T2 oracle, oracle summary, and drift artifacts.
- Published the supported profile scope for PDF/A, PDF/UA, and tracked WTPDF
  profiles.
- Recorded that the first veraPDF-grade PDF/A/PDF/UA claim remains blocked
  until the T3 real-world corpus gate is run and reviewed.
- Linked the release evidence from user and developer documentation.

## Release Evidence

The G8 snapshot is in
[`docs/reviews/m9-g8-readiness-release-hardening.md`](../reviews/m9-g8-readiness-release-hardening.md).

| Gate | Result |
| --- | --- |
| `make standard-gates` | Passed; `cargo deny check` emitted only the documented unmatched-license and duplicate `wit-bindgen` warnings |
| `make parity-profile-report` | Passed; 15 profiles, 6,749 rules, 6,749 lowered, 6,749 bound, 0 unsupported |
| `make parity-model-schema` | Passed |
| `make parity-corpus` | Passed; 13 rows, 13 matches, 0 unexpected drift |
| `make parity-burn-down` | Passed; 0 release-blocking unsupported clusters |
| `make oracle-corpus` with public T2 manifest | Passed; 3 rows, 0 unexpected drift, 0 false-compliant rows |
| `make oracle-corpus-summary` | Passed |
| `make readiness-gates` | Passed with the public T2 manifest and G7 baseline directory |

## Readiness Decision

This is not a public veraPDF-grade readiness release. The T3 real-world corpus
gate has 0 committed release-lab rows in this snapshot. The first claim requires
the release-lab corpus evidence defined by
[`specs/27-oracle-corpus-verification-plan.md`](../../specs/27-oracle-corpus-verification-plan.md).

## Known Drift

All three live public T2 oracle mismatches are classified as
`expectedDriftUnsupportedRule`. In each case pdfv returns `Incomplete`, not
`Valid`, so the rows do not create false-compliant release risk.

## Operator Guidance

Use this snapshot to evaluate profile/rule coverage and to run deterministic
PDF validation where `Incomplete` is an acceptable conservative outcome. Keep
veraPDF in the release gate for arbitrary real-world PDF/A/PDF/UA collections
until a T3 release-lab snapshot passes.
