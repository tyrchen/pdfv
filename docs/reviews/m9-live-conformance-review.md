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

The initial checked-in three-row T2 manifest produced three classified
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

## Rendered Glyph Follow-up

The validator decodes the strings shown by `Tj`, `TJ`, and the quote
operators in the active font and text rendering mode. It carries text state
into invoked Forms, follows annotation appearance streams, and handles
indirect font selections in graphics states. Text bytes stay internal to
validation and are excluded from feature reports. The width check uses the
font dictionary and the embedded program for the same code. Supported paths
include WinAnsi TrueType, Identity-H CID fonts, raw CFF Type 1 fonts, and
Type 3 glyph procedures selected by a Differences encoding.

PDF/A-1 CIDSet validation tests the CIDs used by text-show operators against
the actual font mapping and the decoded bit set. Both checked-in PDF/A-1b
positives now match veraPDF at `Valid`. Clearing the CIDSet bits in a
size-preserving copy of `validDocument.pdf` makes both engines fail the same
rule, `6.3.5-3`. The third checked-in row remains `Invalid` in both engines.

The 25-file PDF/A-4 colour group now matches veraPDF on every decision. In
addition to the width evidence, this required correcting the applicability
of font-subtype, ICC-profile, and COS container checks. The dedicated font
set contains 15 upstream documents and the modified CIDSet control.
Eleven decisions agree with veraPDF, including every positive. The five
incomplete rows exercise legacy multiple-master CFF fonts or embedded CMaps.
None of the comparisons classified an oracle-negative file as valid.

## Embedded CMaps and MMType1 Follow-up

Text shown with an embedded horizontal CMap is decoded according to its
declared code spaces. The validator checks the selected CID against the font
program and the CIDSet. Ordinary CID mappings take precedence over
`.notdef` mappings when the preferred glyph is present. The model checks the
stream's writing mode and maximum declared CID on the active Type 0 font.
Embedded encodings and their `/UseCMap` references have separate validation
scopes. Inherited maps with unresolved bases withhold glyph evidence.
[Adobe CMap specification](https://adobe-type-tools.github.io/font-tech-notes/pdfs/5014.CIDFont_Spec.pdf)

The three PDF/A-1b CMap negatives now agree with veraPDF on rule `6.3.5-3`.
A copy of `6-3-5-t03-fail-b` with its CIDSet enlarged to contain the used CIDs
passes in both engines. The two PDF/A-4 `MMType1` fonts embed instantiated
Type 1C programs. Reading those programs through the existing CFF path
finds the width discrepancies in rule `6.2.10.5-1`. A copy of
`6-2-10-5-t01-fail-b` with corrected dictionary widths passes in both engines.
The supplemental manifest includes all five original negative controls.
All 18 font comparisons and all 25 PDF/A-4 colour comparisons agree with
veraPDF on the document outcome, with zero false-compliant rows.

## PDF/UA Painted Content and Structure Follow-up

PDF/UA content checks operate on actual painting, including text, path
painting, shading, inline images, image XObjects, and the operators inside
invoked Forms. Tagging evidence comes from the owning page or Form's
ParentTree entry and a parent chain that reaches the structure tree root.
An Artifact exemption requires a balanced, valid marked-content sequence.
Invocations of a reused Form are checked independently when their tagging
contexts differ.

Imported structure rules use their applicable roles. The model resolves
namespace role maps and follows `Div`, `Part`, and `NonStruct` wrappers
when finding significant parents, children, and content items.

The live PDF/UA-1 negative now reports both page-content failures for
`7.1-3`. The PDF/UA-2 negative reports the Form-content failure for
`8.2.2-1`. Both match veraPDF at the targeted rule and check count.
Across the four PDF/UA fixtures, the unsupported rule observations dropped
from 347 to 90. The nine supplemental PDF/A fixtures still match veraPDF.

## Unicode Mapping Follow-up

The Unicode presence and forbidden-value rules inspect the character codes
used at text-show operators. The encoding determines code boundaries.
Explicit ToUnicode mappings take precedence over font mappings for the same
code. The validator supports UTF-16BE sequences, range and array declarations,
and collection fallback for Adobe Japan1, GB1, CNS1, Korea1, and KR.
Rule reports retain operator locations and outcomes.
[Adobe CMap specification](https://adobe-type-tools.github.io/font-tech-notes/pdfs/5014.CIDFont_Spec.pdf)

The PDF/UA-2 fixture `8.4.5.8-t01-pass-a` maps its eight used codes through
Adobe-Japan1 even though the font dictionary omits ToUnicode. Both Unicode
rules now pass for all eight codes. The document still has incomplete
assertions outside those rules.

## Release Blockers

- The automatic fallback still selects the ten-rule `pdfv-m4` profile. veraPDF
  selects the full fallback profile. The live comparison marks the two
  executions as incompatible.
- The four supplemental PDF/UA documents remain incomplete. Open work includes
  inherited and predefined non-Identity font encodings, symbolic TrueType
  font evidence, and rules scoped to language strings and file specifications.
- The current workspace has zero private T3 release documents. Qualification
  requires the corpus and acceptance evidence specified in the
  [oracle verification plan](../../specs/27-oracle-corpus-verification-plan.md).
