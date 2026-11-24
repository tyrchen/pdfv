# Rule Check Completion Plan

Owner: pdfv · Baseline: `3ff00fc`

## Acceptance

Every applicable built-in rule needs a `Passed` or `Failed` decision backed by
the rule's actual PDF object, relationship, or operator. Known conforming
documents pass, and violating controls fail the corresponding rule. Public
corpus qualification requires zero runtime unsupported checks on the scoped
files, zero unexpected disagreements, and zero false-compliant decisions.
Private release qualification follows the [T3 oracle gates](./27-oracle-corpus-verification-plan.md#6-release-gates).

The baseline binds 6,749 rules across 15 profiles. The checked-in public T2
manifest has 8 matching decisions. The supplemental manifest has 9 matching
PDF/A decisions and 4 incomplete PDF/UA decisions with 73 unsupported rule
evaluations. Attachment controls already agree on PDF/UA-1 `7.11-1` and
PDF/UA-2 `8.14.1-1`. [Baseline evidence](../docs/reviews/m9-live-conformance-review.md)

## Work order

| Step | Deliverable | Acceptance evidence |
| --- | --- | --- |
| 1 | Scope `CosLang` to language strings exposed by the catalog, reachable structure, annotations and form fields, and marked-content property lists. Reuse bounded PDF text decoding. | Language-rule counts and outcomes match veraPDF on the four supplemental files and positive and negative language controls. Tests cover direct, indirect, named, inline, and inherited use-site distinctions. |
| 2 | Bind `CosActualText`, `CosAlt`, and `CosTextString` to the applicable text fields and check Unicode Private Use Area values. | Matching assertions on positive and negative controls for each scope and encoding. The four supplemental files produce zero unsupported text-property checks. |
| 3 | Finish glyph-width and encoding evidence for the remaining PDF/UA fixtures. Extend coverage for inherited CMaps, non-Identity encodings, and symbolic TrueType based on observed failures. | Both PDF/UA-1 fixtures decide their glyph-width rules from the rendered codes. All four supplemental files match the oracle decision, with zero unsupported checks. PDF/A font and colour controls still agree. |
| 4 | Bind PDF/A-3 `/AF` associations and repeat the rule and corpus audit across every built-in profile, including WTPDF. Add controls for each newly identified model or applicability gap. | `/AF` cases distinguish real associations from name-tree membership or `AFRelationship`. Each closed cluster has rule-level evidence, and the expanded public manifest has zero unsupported checks and zero unexpected differences. |
| 5 | Run adversarial and private-corpus qualification and publish the readiness evidence. | Repository gates pass, release reports contain observed samples and rule counts, and the private T3 inputs meet the size, agreement, mismatch-classification, and false-compliance requirements. |

Steps 1 and 2 share text decoding and use-site traversal. Steps 3 and 4 keep
their own controls and run the existing public manifests after each change.
At every step, record the remaining rule IDs with the reason and representative
fixture. Steps 1 through 4 can run with the public fixtures. Step 5 also needs
the private T3 corpus.

Step 1 exposed a parser prerequisite. The language controls contain linked
incremental revisions and linearized leading sections. Read those sections
through their xref links before evaluating language. Step 4 covers the
remaining audit of active object identity across free entries, compressed
replacements, and hybrid references. [Parser design](./11-parser-core-design.md)

## Step 1 evidence

The language rule agrees with veraPDF on both pass and fail counts across
72 controls from PDF/A-1a, PDF/A-2a, PDF/UA-1, and PDF/UA-2. Each of the four
supplemental PDF/UA files has one passing language assertion. Their aggregate
unsupported count fell from 73 to 14. Twelve events are the alternative,
replacement, and general text rules. Two events are the PDF/UA-1 width rule
`7.21.5-1`. [Live review](../docs/reviews/m9-live-conformance-review.md#language-and-incremental-revisions)

## Step 2 evidence

All 170 rule comparisons across 83 fixture and profile selections match
veraPDF Greenfield 1.30.4. The controls cover PDF/UA-2, WTPDF accessibility
and reuse, PDF/A-4, and combined PDF/A-4 and PDF/UA-2 auto-detection. They
exercise all three Unicode private-use ranges, PDFDocEncoding, direct and
indirect text, named marked-content properties, and reachable outlines.

The four supplemental PDF/UA files have zero unsupported text-property
checks. The PDF/UA-2 positive is `Valid` and the PDF/UA-2 negative is
`Invalid`, in agreement with veraPDF. The expanded public manifest now
matches 11 of 13 decisions with zero false-compliant decisions. The two
PDF/UA-1 files each have one unsupported `7.21.5-1` evaluation because
`widthFromFontProgram` is unavailable for a used font. [Live review](../docs/reviews/m9-live-conformance-review.md#private-use-code-points-in-text-strings)

## Step 3 evidence

Both remaining PDF/UA-1 fixtures now measure glyph widths from their
embedded TrueType character maps and horizontal metrics. The positive
fixture passes all six width assertions. The negative passes all fifteen
width assertions and fails its real-content requirements. Both fixtures
also pass the embedded TrueType `cmap` checks and agree with veraPDF on
document status. The 13-row supplemental manifest and the 8-row checked-in
manifest both have complete outcome agreement, zero unsupported checks,
and zero false-compliant decisions. The 41 additional PDF/A font and
colour controls also match their expected document decisions.
[Live review](../docs/reviews/m9-live-conformance-review.md#built-in-truetype-encoding-and-glyph-widths)

## Step 4 evidence to date

PDF/A-3 attachment rules now use typed file specifications, actual `/AF`
references, and the embedded streams selected by `/EF`. All 581 attachment
assertions across 136 PDF/A-3b controls agree with veraPDF, including the
12 upstream attachment fixtures. The attachment rules produce zero
unsupported evaluations.

Image bit-depth rules distinguish ordinary invocations, stencils, soft masks,
and streams reached through `/Mask`. DeviceN limits and colourant checks use
the colour spaces reached through actual content and group usage. XMP
parsing preserves RDF property scope and checks the fields of each
extension resource. Annotation rules resolve the appearance entries and
inherited widget type. All 12 upstream attachment fixtures now agree
at document level with zero incomplete results and zero unsupported checks.

The broader audit sampled all 15 built-in profiles. Twenty-three of its 30
whole-document decisions agree and seven differ, with zero false-compliant
decisions. The checked-in 8-row and supplemental 13-row release checks
continue to agree.

The dedicated 575-file XMP audit has 165 false-compliant decisions linked
to existing packet-level value-type checks. Bind each XMP property to its
declared or predefined qualified schema, preserve RDF value shape, and
apply primitive, structured, and closed-choice constraints. Verify
metadata attached to reachable objects alongside the catalog packet.
[Live review and representative rules](../docs/reviews/m9-live-conformance-review.md#xmp-extension-resources-and-annotation-appearances)

## Implementation constraints

Use a family for each rule scope and preserve context in assertions. A shared
indirect value can be checked at distinct use sites. Retain text evidence only
inside validation where existing limits permit it. Use the existing typed
parser and validation errors for failed traversals and resource caps. Keep the
model and profile dispatch synchronous and within the session boundary in
[13](./13-validation-engine-design.md). Run `make standard-gates`, `make test`,
and the relevant live oracle comparisons before recording a step as complete.

## References

- [Content operators](./21-content-stream-operator-model-design.md)
- [Font and resource semantics](./22-resource-font-color-semantics-design.md)
- [Structure and accessibility](./23-structure-accessibility-design.md)
- [Readiness implementation](./28-verapdf-grade-validation-impl-plan.md)
- [Remaining review findings](./93-improvements-review.md)
