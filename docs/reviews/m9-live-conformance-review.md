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

## Automatic Fallback Follow-up

Automatic validation uses the generated profile for the configured fallback
flavour when XMP is absent, unreadable, or contains no supported claim. XMP
diagnostics remain in the report. The PDF/A-1b, PDF/A-2, and PDF/A-3 stream
syntax checks accept both LF and CRLF after the `stream` keyword.

The missing XMP and unsupported PDF/A identification controls return `Invalid` with
the generated PDF/A-1b profile in both engines. The LF stream sample has a
supported PDF/A-1b claim and passes in both engines.

The PDF/A-2 and PDF/A-3 size rule applies to the five named page boundaries.
`MediaBox` and `CropBox` can be inherited. `BleedBox`, `TrimBox`, and `ArtBox`
are read from the page. The T2 PDF/A-2b controls exercise a 100-unit
`MediaBox` that passes and a 2-unit `MediaBox` that fails `6.1.13-11`.

## PDF/UA File Specifications

File specification rules for PDF/UA and WTPDF use typed COS dictionaries and
entries in the catalog's `EmbeddedFiles` name tree. A typed entry in that
tree receives both evaluations. PDF/UA-1 checks that an embedded file has
nonempty string values for `F` and `UF`. PDF/UA-2 and WTPDF require `Desc` on
name-tree entries.

veraPDF Greenfield 1.30.4 and pdfv agree on the pass and fail counts for all
four targeted upstream files. The two PDF/UA-1 negatives each have two
failures for `7.11-1`, and the positive has two passes. The PDF/UA-2 negative
has one pass for the physical dictionary and one failure for its name-tree
entry under `8.14.1-1`. [PDF/UA-1 fixtures](https://github.com/veraPDF/veraPDF-corpus/tree/bb75f4f0073d9350dfd058c0162a367e6fadf25e/PDF_UA-1/7.11%20Embedded%20files) · [PDF/UA-2 fixture](https://github.com/veraPDF/veraPDF-corpus/blob/bb75f4f0073d9350dfd058c0162a367e6fadf25e/PDF_UA-2/8.14%20Use%20of%20embedded%20files/8.14.1%20Descriptions%20for%20embedded%20files/8.14.1-t01-fail-a.pdf)

The original four supplemental PDF/UA fixtures now have 73 unsupported
evaluations, down from 90. All four document decisions remain `Incomplete`.

## Language and Incremental Revisions

Language rules read the catalog, reachable structure elements, ParentTree
targets of annotations and AcroForm fields, and `BDC` property lists. Language
strings use PDFDocEncoding, UTF-16BE, or UTF-8 with a byte-order mark. The
profile determines whether empty tags or digits in subtags are permitted.
Reused content is checked again when it resolves to different language
evidence. Greenfield 1.30.4 keeps the first resource context for a reused
stream within each stream model type.

The parser follows linked incremental sections across intermediate `%%EOF`
markers and handles the leading EOF in the tested linearized PDFs. It keeps
post-EOF evidence for unlinked trailing data. Complete active-object precedence
for free and compressed entries remains in the parser audit.

| Profile | Language controls with matching assertion counts |
| --- | ---: |
| PDF/A-1a | 11 / 11 |
| PDF/A-2a | 9 / 9 |
| PDF/UA-1 | 26 / 26 |
| PDF/UA-2 | 26 / 26 |

Each of the four supplemental PDF/UA files now reports one passing language
assertion. The total unsupported count fell from 73 to 14. The document
decisions remain `Incomplete` in these four files.

## Private-Use Code Points in Text Strings

The PDF/UA-2 and WTPDF rules read `ActualText` and `Alt` on reachable
structure elements and marked-content sequences in the semantic content view.
PDF/A-4 also checks replacement text in ordinary content streams and
marked-content points. Automatic selection of PDF/A-4 and PDF/UA-2 makes
both content views available to the applicable rules. A reachable outline's
`Title` and each scoped `Lang` string receive the readable-text check.

veraPDF Greenfield 1.30.4 and pdfv agree on all 170 pass and fail count
comparisons across 83 fixture and profile selections. This includes four
passing readable-text assertions on each of the two original supplemental
PDF/UA-2 fixtures and three passes and one failure on the upstream
`8.6-t01-fail-a` fixture. [PDF/UA-2 readable-text fixture](https://github.com/veraPDF/veraPDF-corpus/blob/bb75f4f0073d9350dfd058c0162a367e6fadf25e/PDF_UA-2/8.6%20Text%20string%20objects/8.6-t01-fail-a.pdf)

Separate edge probes found three Greenfield limitations. It keeps the first
property binding when a stream is shared across pages, skips a valid code
unit following a malformed UTF-16BE high surrogate, and can abort on an
indirect string in a named marked-content property dictionary. pdfv retains
both property outcomes, recovers the following code unit, and resolves the
indirect string. The regression suite covers these paths.

The expanded public manifest now matches 11 of 13 document decisions. The
PDF/UA-2 positive passes and the PDF/UA-2 negative fails on real content.
Both agree with veraPDF and have zero unsupported rules. Each supplemental
PDF/UA-1 file still has one unsupported glyph-width rule.

## Built-In TrueType Encoding and Glyph Widths

Simple TrueType fonts with built-in encodings resolve rendered character
codes through their embedded `cmap` tables. The width comparison uses
horizontal advances scaled by units per em, the `FirstChar` to `LastChar`
dictionary range, and `MissingWidth` for codes outside that range.
The TrueType program rules read `cmap` encoding records from embedded
programs used by text-show operators. They count each font dictionary once,
including a font reached through both a page and an invoked Form.

| PDF/UA-1 supplemental fixture | Passing width checks | Document outcome in both engines |
| --- | ---: | --- |
| `7.2-t17-pass-d` | 6 | Valid |
| `7.1-t03-fail-b` | 15 | Invalid |

Both documents have zero unsupported rules and each passes both TrueType
program checks. The expanded public manifest is 13/13 and the checked-in
manifest is 8/8, with zero incomplete or false-compliant decisions. The
41 PDF/A font and colour controls also agree on document outcome.

The three targeted PDF/UA-1 rules agree on pass and fail counts in 132 of
135 comparisons across 45 controls. In two controls, pdfv records a passing
width assertion in both the page and Form stream. Greenfield counts the
shared glyph once. The corrupt-font control still produces unavailable
width evidence in pdfv. Greenfield records a pass with a null font width.
See [the review backlog](../../specs/93-improvements-review.md#open-m9-findings).

## PDF/A-3 Associated Files and Profile Audit

A file specification's association comes from the resolved member of an
`/AF` array on a reachable owner or the array referenced by an executed
`/AF /name BDC` mark. An `AFRelationship` name and a name-tree entry
receive independent checks. Attachment MIME checks read the streams selected
by `/EF`, including streams that omit `/Type /EmbeddedFile`. Several
selectors can point to one stream, which receives one MIME check.

veraPDF Greenfield 1.30.4 and pdfv agree on all PDF/A-3b `6.8-1` through
`6.8-4` assertion counts across 136 controls. The matching evidence comprises
581 individual checks with zero unsupported evaluations for these rules.
The controls include 12 upstream fixtures, indirect aliases, reachable and
detached resources, and marked content in pages, Forms, appearances, tiling
patterns, and Type 3 glyph programs. Four of the 12 upstream fixtures also
match at document level. The other eight expose rules outside clause 6.8.
[Pinned PDF/A-3b attachment fixtures](https://github.com/veraPDF/veraPDF-corpus/tree/bb75f4f0073d9350dfd058c0162a367e6fadf25e/PDF_A-3b/6.8%20Embedded%20files)

A separate 30-case audit sampled all 15 built-in profiles, using two
controls per profile. Twenty document outcomes match. Six are incomplete
and four are incorrectly invalid. The audit found zero false-compliant
decisions. These are the representative gaps for the next rule-scope work.

| Profile | Representative control | Rules needing evidence or scope |
| --- | --- | --- |
| PDF/A-1a | `6-3-8-t01-pass-a`, `6-3-8-t01-fail-a` | Font width `6.3.6-1`, Unicode `6.3.8-1`, and CIDSet `6.3.5-2` |
| PDF/A-2a | `6-2-11-7-3-t01-pass-a`, `6-2-11-7-3-t01-fail-a` | UTF-8 names `6.1.8-1` |
| PDF/A-2b | `6-1-12-t01-pass-a` | Digital signature applicability `6.4.3-1` through `6.4.3-3` |
| PDF/A-2u | `6-2-11-7-2-t01-pass-a`, `6-2-11-7-2-t01-fail-a` | Width, Unicode, and CMap rules `6.2.11.5-1`, `6.2.11.7.2-1/-2`, `6.2.11.4.2-1` |
| PDF/A-3b | Upstream `6-8-t02-*` attachment fixtures | XMP `6.6.2.3.1-1/-2` and `6.6.2.3.2-1`, annotation appearance `6.3.3-1` |
| PDF/A-4e, PDF/A-4f | `6-2-4-1-t01-pass-a`, `6-3-1-t01-pass-a` | Annotation appearance `6.3.3-1` |

## Image Component Depth and DeviceN Scope

The base image bit-depth rule uses invoked images and inline images. The
hard-mask rule checks the stream selected by an invoked image's `/Mask`
entry. In 68 controls across PDF/A-1b, 2b, 3b, and 4, 524 of 544 rule
comparisons agree with veraPDF. The remaining 20 comparisons cover five
malformed bit-depth values. pdfv requires integer COS values in these
checks. veraPDF coerces those malformed values. These rules produced
zero unsupported evaluations in the controls.

DeviceN rules use the colour-space arrays reached through content, images,
shadings, and transparency groups. They count component slots and check the
colourant names given by `/Colorants` and `/Process`. Graphics state carries
the original default gray resource scope into forms that paint with it.
All 294 DeviceN rule comparisons across 49 controls and the same four
profiles agree with veraPDF, with zero unsupported evaluations.

The 12 upstream PDF/A-3b attachment fixtures now agree on 8 document
decisions, up from 4. All 12 have a definitive result with zero unsupported
evaluations. The four remaining false-invalid decisions are
`6-8-t02-pass-c`, `-d`, `-e`, and `-g`. They share the XMP extension
schema rule failures listed above. `-g` also fails the annotation appearance
rule. The attachment checks still agree on all 581 assertions.

The 30-case audit now has 21 matching outcomes, six incomplete decisions,
and three false-invalid decisions, with zero false-compliant decisions.

## XMP Extension Resources and Annotation Appearances

XMP property extraction starts at the first RDF root. XML wrapper
attributes such as `x:xmptk` stay outside RDF validation. The extension
schema rule checks each declared schema, property, value type, and field
against its allowed immediate RDF fields. Typed and untyped resource nodes
retain their declaration scope. The extension schema rule agrees on
573 of 575 PDF/A-2b fixtures and all eight additional scope controls.
The other two fixtures include object-level metadata streams beyond the
catalog packet currently parsed by pdfv.

The annotation rules check the resolved `/AP` dictionary, the `/N` entry
type, and the inherited widget `/FT`. All 465 appearance-rule comparisons
agree across 31 fixtures and four PDF/A profiles. On 14 additional controls,
55 of 56 comparisons agree. pdfv requires four entries in `/Rect` and
veraPDF tolerates an extra entry in this control. Device colour checks now
reach the stream linked by a 3D annotation. All 75 comparisons agree across
25 PDF/A-4e fixtures covering page resources, output, and transparency.
The representative negative leaves a DeviceRGB violation after its
appearance passes.

All 12 upstream PDF/A-3b attachment documents now match their expected
outcomes, including the four previously false-invalid cases. They produce
zero unsupported evaluations, and the wider attachment corpus still
matches all 581 assertions. The 30-case audit across 15 profiles now has
23 matching outcomes, six incomplete decisions, and one false-invalid
decision, with zero false-compliant decisions. The last false-invalid
decision is the PDF/A-2b permissions control.

The 575-fixture PDF/A-2b XMP audit still has 302 matching document outcomes,
165 false-compliant decisions, and 108 false-invalid decisions. A rebuild
of `7a91f42` yields the same decision for every file in this audit.
All 165 false-compliant cases involve the XMP property value-type rule
`6.6.2.3.1-2`. The property rules still aggregate evidence at packet
level. The remaining work requires individual qualified properties
linked to the applicable predefined or declared types and lexical,
structural, and closed-choice validation.
[Pinned PDF/A-2b corpus](https://github.com/veraPDF/veraPDF-corpus/tree/bb75f4f0073d9350dfd058c0162a367e6fadf25e/PDF_A-2b)

## Qualified XMP Property Validation

The XMP property rules evaluate qualified RDF properties individually. The
validator checks the scalar, array, or structure against the applicable
PDF/A predefined or packet-declared type. Scalar checks preserve lexical
whitespace and resolve XML character references under the packet limits.
XMP thumbnails retain their type evidence without retaining image payloads.
XPath values receive bounded XPath 1.0 syntax compilation. The comparison
uses Greenfield's default schema settings. Independent review verified
449 profile-expanded property
definitions and 165 structure entries against Greenfield 1.30.4.

All 575 PDF/A-2b XMP fixtures now match at document level, up from 302.
The prior 165 false-compliant and 108 false-invalid decisions are gone
in this corpus. All 12 upstream PDF/A-3b attachment fixtures also agree,
including all 36 comparisons of their three XMP schema rules. The
specialized corpora show zero unsupported evaluations. The PDF/A-2b XMP
rules match in 1,717 of 1,725 comparisons. The eight differences across
three fixtures come from object-level metadata packets.

Focused controls retain differences outside these corpora. pdfv requires a
language qualifier on each language-alternative item. XPath function
resolution and arity checks remain open. The PDF/A identification rule
still uses the normalized conformance claim. These
differences affect, respectively, mixed language arrays, syntactically
correct XPath with invalid function calls, and whitespace around
`pdfaid:conformance`.

## Object-Level XMP Packets

PDF/A-2, 3, and 4 validate metadata streams linked through a dictionary's
`/Metadata` entry. The walk includes nested COS dictionaries and stream
dictionaries. It resolves the link to a `/Type /Metadata /Subtype /XML`
stream and checks each stream once. The catalog remains the source for
document identification and profile selection. Packet-level rules cover
XML serialization, encoding, and packet header attributes. Object packets
can use catalog extension declarations, with local definitions taking
precedence for the same qualified property. The decoder applies both a
per-stream limit and a shared budget for decoded packet bytes.

All 575 PDF/A-2b XMP documents still match Greenfield at document level.
The three directly audited rules now match in **1,725 of 1,725**
comparisons, with zero unsupported evaluations. The three fixtures
with object metadata also match all 63 comparisons across the 21
schema rules. The 12 PDF/A-3b attachment fixtures continue to match
all 84 comparisons of their attachment and targeted XMP rules.

In shared-stream controls, pdfv keeps catalog identification active when
the catalog packet is also referenced by an object. Distinct packets each
validate their own value when they share a qualified property name.
Greenfield can coalesce these observations during traversal, which makes
its assertions sensitive to packet order.

## Digital Signatures and Document Permissions

PDF/A-2 and PDF/A-3 signature rules start at the catalog's AcroForm field
tree. They select the dictionary in a signature field's own `/V`, including
fields that inherit `/FT /Sig` from their parent. A usage-rights signature
whose only reference is `/Perms/UR3` produces zero digital-signature
assertions. Each applicable `/Reference` entry is evaluated under the
DocMDP restriction with the catalog's permissions dictionary as evidence.
Null-valued permission and signature-reference entries represent absence.

The byte-range check matches the excluded gap to the source token that
holds `/Contents` and checks the signed end against the applicable revision's
`%%EOF`. The parser permits one optional final line ending and reports
any bytes after it. For PKCS#7, the validator decodes DER SignedData and
checks for parseable X.509 certificates and the number of SignerInfo
records. The rule evidence covers structure, certificate presence, and byte
coverage. [CMS SignedData](https://www.rfc-editor.org/rfc/rfc5652.html#section-5.1)

The supplemental oracle manifest now contains all six upstream PDF/A-2b
permission fixtures. Their document decisions and all 30 targeted rule
comparisons agree with Greenfield 1.30.4. The broader 15-profile audit
now matches 24 of 30 document decisions. Six decisions remain incomplete
and there are zero false-compliant or false-invalid decisions in that audit.

Independent review compared 296 targeted pass/fail counts across 58
fixtures. All 292 matching comparisons and the four differences produced
definitive rule outcomes. pdfv requires exactly two byte ranges, accepts
a gap that precisely selects an indirect Contents string, and requires
any padding after the DER object to be zero. The nonzero-padding fixture
is the sole document decision difference in this focused set.

## UTF-8 Names and Glyph Replacement Text

The PDF/A-2, PDF/A-3, and PDF/A-4 name rules evaluate original COS name
bytes after PDF escape expansion. They cover BaseFont names in resource
slots and fonts used by text-show operators, Separation and DeviceN
colourants, and reachable structure roles and RoleMap entries. Colour
evidence includes effective resources, paint operations, nested colour
spaces, transparency groups, invoked images and alternate images. Shared
colour spaces with resource-dependent links retain the context used to
resolve their colourants.

PDF/A-2a and PDF/A-3a now evaluate whether each used glyph mapped to a
Unicode Private Use Area value has a string-valued ActualText
replacement. The check uses that glyph's Unicode mapping, its enclosing
marked-content sequence, or the structure element selected through the
ParentTree. An explicit inner MCID shadows an outer MCID. The check
retains the enclosing marked-content replacement through Form calls
and also inspects marked content local to the Form or appearance.

Ten upstream PDF/A-2a glyph fixtures and two PDF/A-2b name fixtures now
match Greenfield on all 12 document decisions and all 22 targeted
rule comparisons. The two PDF/A-2b files were false compliant on the
previous build. The supplemental oracle manifest now includes all 12
fixtures, and the 15-profile audit matches 26 of 30 document decisions.
The four remaining controls are incomplete. The 575 PDF/A-2b XMP
documents and the 12 PDF/A-3b attachment documents retain complete
agreement in their targeted checks.

Focused synthetic comparisons record differences where Greenfield
converts used font names through a Java string, coalesces identical
inline structure elements or resource-dependent colour links, or
omits the colour space under an uncoloured Pattern. For replacement
text it treats a Form as a separate marked-content scope. The pdfv
model uses original PDF name bytes and the context of the Form call.

## Type 1 Font Subsets and Macintosh Character Maps

Used TrueType glyphs with MacRomanEncoding resolve through the Windows
Unicode, Macintosh Roman, and glyph-name tables. Type 1 glyphs use the
PDF's selected encoding, including Differences, and can fall back to
the embedded program's encoding. A bounded interpreter reads horizontal
advances from the Type 1 charstrings and their subroutines. The decoded
FontFile stream supplies binary eexec bytes using Length1 and Length2.
[ISO 32000-1, sections 9.6.6 and 9.9](https://opensource.adobe.com/dc-acrobat-sdk-docs/standards/pdfstandards/pdf/PDF32000_2008.pdf)

The Type 1 CharSet rule checks the glyph names used by PDF/A-1 text
operations. PDF/A-2 and PDF/A-3 compare the declared names against the
embedded program's glyph inventory, excluding the reserved .notdef glyph.
PDF/A-1 evaluates Unicode coverage per glyph and recognizes exact
Adobe Glyph List names for Type 1 fallback. The vendored list includes
all names in the PDF Symbol font.
[Pinned Adobe Glyph List](https://github.com/adobe-type-tools/agl-aglfn/blob/4036a9ca80a62f64f9de4f7321a9a045ad0ecfd6/glyphlist.txt)

The four previously incomplete PDF/A-1a and PDF/A-2u documents match
all 14 targeted rule counts and all four document outcomes. Each negative
retains a Unicode mapping failure for CMEX10's integraldisplay glyph.
All **30 of 30** document outcomes in the 15-profile audit now match,
with zero incomplete, false-compliant, or false-invalid decisions.
The public manifests match 8 of 8 and 35 of 35 document outcomes.

Independent review matched 131 of 132 rule counts across 37 files.
One focused PDF/A-1 control differs. When a PDF omits its Encoding entry
and declares an empty CharSet, pdfv checks the glyph name from the
embedded encoding and fails its CharSet rule. Greenfield records a pass.

## Active Cross-Reference Entries

The parser builds the object store from the terminal `startxref` and linked
sections. A new entry can free an object, advance its generation, or replace
its storage with an object-stream member. Hybrid selection checks the current
classic table before its supplemental stream and then follows the classic
`/Prev`. Linearized files use the logical trailer order and ignore the
first-page end marker when recording signed revisions.
[ISO 32000-1, sections 7.5.4–8 and Annex F](https://opensource.adobe.com/dc-acrobat-sdk-docs/standards/pdfstandards/pdf/PDF32000_2008.pdf)

All parsed trailer dictionaries remain available. Metadata, encryption, and
validation read the primary chain even when a supplemental stream or an
abandoned revision has its own dictionary.

The parser reports a missing object when a live offset or compressed member
index cannot be resolved. The compressed free-entry and mismatched-index
controls expose Greenfield differences. It can retain the old object for a
stream free entry and resolve a compressed member by number at the wrong index.
The parser reads an unencrypted xref stream before looking up the active
encryption dictionary. The AESV2 regression exercises password-based
decryption and password-free detection through a stream trailer.

All 43 decisions across the two public manifests match Greenfield 1.30.4.
The 15-profile audit retains 30 matching outcomes and zero incomplete
decisions. The PDF/A-2b XMP and PDF/A-3b attachment corpus replays agree on
all 587 document outcomes and 1,809 selected rule counts.

## Release Blockers

- The focused XPath function and identification cases still need
  rule-level evidence.
- The current workspace has zero private T3 release documents. Qualification
  requires the corpus and acceptance evidence specified in the
  [oracle verification plan](../../specs/27-oracle-corpus-verification-plan.md).
