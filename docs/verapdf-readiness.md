# veraPDF Readiness

Status: evidence packaged, release-grade claim withheld.

This page records the public readiness scope for pdfv's veraPDF-grade validation
work. "veraPDF-grade" in this project means decision-grade agreement with
veraPDF for in-scope PDF/A and PDF/UA workflows, not endorsement by the veraPDF
project and not byte-identical veraPDF reports.

## Supported Profile Scope

The G8 release-hardening snapshot imports, lowers, and binds every rule in the
vendored profile set used by pdfv:

| Profile family | Flavours in the snapshot |
| --- | --- |
| PDF/A | `pdfa-1a`, `pdfa-1b`, `pdfa-2a`, `pdfa-2b`, `pdfa-2u`, `pdfa-3a`, `pdfa-3b`, `pdfa-3u`, `pdfa-4`, `pdfa-4e`, `pdfa-4f` |
| PDF/UA | `pdfua-1`, `pdfua-2-iso32005` |
| WTPDF | `wtpdf-1-0-accessibility`, `wtpdf-1-0-reuse` |

The first readiness claim is scoped to PDF/A and PDF/UA only. WTPDF profile
data is tracked for parity, but WTPDF is not part of the first public
veraPDF-grade readiness claim.

## Evidence Snapshot

Current release-hardening evidence is stored in
[`docs/reviews/m9-g8-readiness-release-hardening.md`](reviews/m9-g8-readiness-release-hardening.md).

| Evidence | Result |
| --- | ---: |
| Imported profiles | 15 |
| Imported official rules | 6,749 |
| Lowered rules | 6,749 |
| Bound rules | 6,749 |
| Unsupported rules | 0 |
| Release-blocking unsupported clusters | 0 |
| Java-free parity corpus rows | 13 |
| Java-free parity corpus unexpected drift | 0 |
| Live T2 public oracle rows | 3 |
| Live T2 unexpected drift | 0 |
| Live T2 false-compliant rows | 0 |
| Live T3 release-lab rows | 0 |

## Known Drift

The committed live T2 public oracle rows currently classify three mismatches as
`expectedDriftUnsupportedRule`: veraPDF returns `Valid` or `Invalid`, while pdfv
returns `Incomplete`. This is intentionally not a false-compliant result.

No live T2 row is classified as `unexpectedDrift`, `pdfvBug`, or
false-compliant in the G8 snapshot.

## Release Blocker

pdfv must not yet claim veraPDF-grade readiness for arbitrary real-world
PDF/A/PDF/UA workflows. The private T3 real-world corpus gate has not been run:

- required release gate: at least 95% outcome-class match on in-scope T3 rows;
- required release gate: 100% mismatch classification;
- required release gate: 0 false-compliant rows;
- default first-claim corpus threshold: at least 1,000 T3 documents unless a
  key decision records a smaller statistically acceptable corpus.

Until that evidence exists, use pdfv as a deterministic Rust validator with
strong parity metrics and conservative `Incomplete` reporting for unsupported
or unproven semantics. Do not present it as a release-grade replacement for
veraPDF on arbitrary private collections.

## Out Of Scope For The First Claim

- byte-for-byte compatibility with veraPDF XML, HTML, or text reports;
- legal certification or endorsement by the veraPDF project;
- WTPDF readiness;
- GUI, server mode, installer, ZIP input, and progress-display parity;
- pixel rendering, visual diffing, OCR, and assistive-technology simulation;
- full PKI trust-chain validation for digital signatures.
