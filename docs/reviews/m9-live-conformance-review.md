# M9 Live Conformance Review

The release comparison now checks the selected profiles as well as the outcome.
It requires complete veraPDF JSON and the corresponding exit code, and bounds
both output streams while the process runs. Release mode verifies fresh public T2
evidence before the private T3 run. The private gate requires 1,000 distinct
in-scope documents, 95% outcome agreement, and zero false-compliant rows.

The PDF/A binary-comment rule now uses the original bytes after the header, and
the classic xref rules check the actual line ending and subsection separator.
On `veraPDFtest-pass-a.pdf`, the PDF/A-1b rule failures for the binary comment
(`6.1.2-2`) and the xref separator (`6.1.4-2`) are gone.

## Live Evidence

The checked-in three-row T2 manifest still produces three classified
`Incomplete` differences with veraPDF Greenfield 1.30.4. An additional run used
eight fixtures at revision `bb75f4f0073d9350dfd058c0162a367e6fadf25e` of the
[veraPDF conformance corpus](https://github.com/veraPDF/veraPDF-corpus/tree/bb75f4f0073d9350dfd058c0162a367e6fadf25e).
The manifest records the exact source paths.

| Profile | veraPDF valid | veraPDF invalid | pdfv incomplete |
| --- | ---: | ---: | ---: |
| PDF/A-1b | 1 | 1 | 2 |
| PDF/A-4 | 1 | 1 | 2 |
| PDF/UA-1 | 1 | 1 | 2 |
| PDF/UA-2 | 1 | 1 | 2 |

The initial run found runtime unsupported-rule observations on all eight
supplemental rows and zero comparable false-compliant rows. The oracle exits
with a failure and writes the artifacts for review.

Run the supplemental manifest against a checkout of the pinned corpus.

```bash
PDFV_VERAPDF_BIN=/path/to/verapdf \
  PDFV_ORACLE_CORPUS_ROOT=/path/to/veraPDF-corpus \
  PDFV_ORACLE_CORPUS_MANIFEST=tests/oracle-t2-upstream-conformance.yml \
  PDFV_ORACLE_OUTPUT_DIR=target/parity/t2-upstream \
  make oracle-corpus
```

## Stream Dispatch Follow-up

Metadata and content wrappers can refer to a stream already visited by the
physical COS stream model. The validator now runs stream syntax rules on that
physical model once. This removed the stream property errors on both PDF/A-4
samples. The negative sample now agrees with veraPDF at `Invalid`. The
positive sample returns `Invalid` with 28 failed checks, so its remaining
assertions need rule-level comparison. The two PDF/A-1b rows each dropped from
30 unsupported evaluations to 26.

## Physical COS Primitive Follow-up

Integer, real, string, and name checks now execute on their respective parsed
types. The walk includes dictionary keys and compressed indirect objects.
Array positions and dictionary entries receive distinct report locations.
String length uses decoded bytes. Hexadecimal syntax rules use the original
digit count, including an odd final digit before padding. Serialized parsed
strings with syntax evidence use a `bytes` and `syntax` object. Legacy byte
arrays remain readable.

The checked-in T2 set reports one matching invalid decision and two incomplete
decisions. Unsupported evaluations on the two PDF/A-1b positive files fell
from 37 and 42 to 1 and 2. The remaining errors require font-program width and
CIDSet evidence. On the eight supplemental rows, the four PDF/A cases now
complete. Both negative cases agree with veraPDF, and both positive cases give
incorrect `Invalid` decisions. The four PDF/UA cases remain `Incomplete`.

## Rule Scope Follow-up

PDF/A extension-schema rules now use the declared RDF container and the
individual schema, property, value type, and field resources. A document can
omit the extension-schema container. A declaration placed in a scalar
attribute fails the container rule.

Rendering intents, filters, blend modes, halftones, and device colour spaces
use model objects scoped to their PDF relationships. Page content-stream
arrays are read as a single operator sequence during validation. Invoked forms
and tiling patterns resolve named resources in the page context, with local
resources taking precedence. Indirect-object spacing rules inspect the
original separators and line endings of physical objects. A usable standalone
xref table or stream restricts the check to its in-use objects.

| Profile | Matching positive decisions | Matching negative decisions | Incomplete decisions |
| --- | ---: | ---: | ---: |
| PDF/A-1b | 1 | 1 | 0 |
| PDF/A-4 | 1 | 1 | 0 |
| PDF/UA-1 | 0 | 0 | 2 |
| PDF/UA-2 | 0 | 0 | 2 |

On the supplemental public set, the PDF/A-4 CalRGB default fixture first
exposed three colour-space false failures. It now passes. The PDF/A-4
indirect-object fixture fails its expected rule, `6.1.8-1`. Five additional
PDF/A-4 negative controls from the same pinned corpus, `6-2-4-3-t01-fail-a`
through `fail-e`, all fail their expected DeviceRGB rule, `6.2.4.3-2`.

The full 25-file `6-2-4-3-t01` PDF/A-4 group has 18 matching decisions,
including three positives and 15 negatives. The four-component ICC rules
apply only to CMYK ICCBased spaces, allowing the RGB/calibrated positives
`pass-c` and `pass-d` to complete. The remaining seven documents have one
unsupported rule each, all for glyph widths in rule `6.2.10.5-1`. There are
zero false-compliant rows in these comparisons.

## Release Blockers

- The automatic fallback still selects the ten-rule `pdfv-m4` profile. veraPDF
  selects the full fallback profile. The live comparison marks the two
  executions as incompatible.
- The two vendored PDF/A-1b positives remain incomplete. Rule `6.3.6-1` needs
  actual glyph widths for rendered character codes, and rule `6.3.5-3` also
  needs to check used CIDs against the embedded subset's CIDSet.
  The fixtures use WinAnsi TrueType and Identity-H CID TrueType. The Identity-H
  subset has 1,674 glyph slots and 19 CIDSet bits, so a glyph-slot count cannot
  establish which rendered characters the CIDSet covers.
- The four supplemental PDF/UA documents remain incomplete. The negative
  controls exercise whether individual page and Form content items are tagged
  or marked as artifacts. The positive controls also need more complete
  structure and text semantics.
- The current workspace has zero private T3 release documents. Qualification
  requires the corpus and acceptance evidence specified in the
  [oracle verification plan](../../specs/27-oracle-corpus-verification-plan.md).
