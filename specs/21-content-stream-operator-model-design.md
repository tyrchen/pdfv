# 21-content-stream-operator-model-design: Content Stream Operator Model

Status: draft · Owner: pdfv · Depends on: [15-parser-filter-source-parity-design.md](./15-parser-filter-source-parity-design.md), [17-validation-model-parity-design.md](./17-validation-model-parity-design.md)

## 1. Purpose

This spec closes the drift called out in [../docs/reviews/verapdf-pdfv-core-drift-review.md](../docs/reviews/verapdf-pdfv-core-drift-review.md): pdfv currently has parser-level stream handling but no veraPDF-like content stream operator model. It owns bounded operator parsing, graphics/text state summaries, marked-content facts, XObject invocation facts, and operator-level model objects. It does not own full rendering, font glyph shaping, color science, or accessibility tree reconstruction.

veraPDF exposes many operator wrappers under `vendors/veraPDF-validation/validation-model/src/main/java/org/verapdf/gf/model/impl/operator`, including base operators, text-show operators, text state, text positioning, marked content, color, graphics state, clipping, path construction/painting, inline image, shading, XObject, and Type 3 font operators.

## 2. Interface

The operator model is internal to `pdfv-core` and feeds [17-validation-model-parity-design.md](./17-validation-model-parity-design.md).

```rust
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ContentStreamSummary {
    pub source: ObjectKey,
    pub operators_seen: u64,
    pub facts: Vec<OperatorFact>,
    pub marked_content: Vec<MarkedContentSpan>,
    pub resource_uses: Vec<ResourceUse>,
    pub warnings: Vec<ValidationWarning>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum OperatorFact {
    TextState { op: OperatorName, location: ObjectLocation },
    TextShow { bytes: BoundedBytes, location: ObjectLocation },
    MarkedContent { tag: PdfName, properties: Option<ObjectKey>, location: ObjectLocation },
    GraphicsState { op: OperatorName, location: ObjectLocation },
    Color { op: OperatorName, operands: BoundedOperands, location: ObjectLocation },
    XObjectInvoke { name: PdfName, location: ObjectLocation },
    InlineImage { width: Option<u64>, height: Option<u64>, filters: Vec<PdfName>, location: ObjectLocation },
    Path { op: OperatorName, location: ObjectLocation },
    Unknown { op: OperatorName, operand_count: u16, location: ObjectLocation },
}
```

`ContentStreamSummary` is built lazily per page/form/type3 glyph stream and cached in `ValidationSession`. Rule evaluation sees facts through model properties and links, never by reading raw stream bytes directly.

## 3. Operator Coverage

| Slice | Required operators | Parity outcome |
| --- | --- | --- |
| Text object/state | `BT`, `ET`, `Tc`, `Tw`, `Tz`, `TL`, `Tf`, `Tr`, `Ts` | Rules can detect text-object boundaries, text-state use, and referenced font resources. |
| Text positioning/show | `Td`, `TD`, `Tm`, `T*`, `Tj`, `TJ`, `'`, `"` | Rules and feature extraction can count/show text operations without exposing text content by default. |
| Marked content | `BMC`, `BDC`, `EMC`, `MP`, `DP` | PDF/UA and WTPDF rules can reason about marked-content presence, tags, and property references. |
| Graphics state | `q`, `Q`, `cm`, `w`, `J`, `j`, `M`, `d`, `ri`, `i`, `gs` | Resource usage and graphics-state facts become available. |
| Color/shading | `CS`, `cs`, `SC`, `SCN`, `sc`, `scn`, `G`, `g`, `RG`, `rg`, `K`, `k`, `sh` | Color-space and shading resource references are visible to [22-resource-font-color-semantics-design.md](./22-resource-font-color-semantics-design.md). |
| Path/clip/paint | `m`, `l`, `c`, `v`, `y`, `h`, `re`, `S`, `s`, `f`, `F`, `f*`, `B`, `B*`, `b`, `b*`, `n`, `W`, `W*` | Conformance rules can detect graphical content and clipping usage. |
| Images/XObjects | `BI`, `ID`, `EI`, `Do` | Inline-image facts and named XObject invocations are available. |
| Compatibility/unknown | `BX`, `EX`, any unknown operator | Unknown operators are facts, not parser panics. |

## 4. Parsing Rules

- Decode stream filters through the registry defined in [15-parser-filter-source-parity-design.md](./15-parser-filter-source-parity-design.md) before operator parsing.
- Token parsing is byte-oriented and rejects only unrecoverable stream syntax; malformed recoverable operators produce `OperatorFact::Unknown` plus warnings.
- Operands are capped by count and serialized byte length. Large strings are represented as redacted length facts unless a later feature explicitly needs decoded text.
- Parsing uses an explicit stack for graphics state with `max_graphics_state_depth`.
- Type 3 charproc streams are parsed under separate `max_type3_charproc_streams` and `max_type3_charproc_ops` limits.

## 5. Model Integration

The following model object families are added or expanded:

| Object type | Properties | Links |
| --- | --- | --- |
| `ContentStream` | `nrOperators`, `hasText`, `hasMarkedContent`, `hasInlineImage`, `hasUnknownOperators` | `operators`, `usedResources` |
| `Operator` | `op`, `family`, `operandCount`, `location`, `isUnknown` | `resource`, `markedContentProperties` |
| `MarkedContent` | `tag`, `hasProperties`, `nestingDepth` | `propertiesObject` |
| `InlineImage` | `width`, `height`, `filters`, `colorSpace`, `bitsPerComponent` | none |

Generated profile coverage must group unsupported rules by missing operator family when these objects are absent or partial.

## 6. Invariants

- Operator parsing never includes raw PDF string content in reports unless the caller explicitly opts into a future text-extraction feature.
- Every operator fact has a deterministic `ObjectLocation`.
- Unknown operators preserve name and operand count under byte caps.
- Resource references from `Tf`, color operators, `gs`, `sh`, and `Do` are normalized into `ResourceUse` entries.
- Operator summaries are bounded by `ResourceLimits.max_content_stream_ops`; overflow produces a warning and incomplete validation status for rules that need omitted facts.

## 7. Tests

- Unit tests for every operator family and malformed operand recovery.
- Generated fixtures for page content, form XObject content, Type 3 charproc content, marked content, inline images, and unknown operators.
- Regression tests that official profile rules referencing operator objects bind to known schema entries.
- Benchmarks for a page with 10k simple operators and a malicious stream with many tiny operands.

## 8. AGENTS.md Binding

- Error Handling: malformed streams return `ParseError` only when unrecoverable; recoverable operator defects become facts/warnings.
- Safety & Security: no `unsafe`, no unbounded recursion, checked arithmetic for operand counts and byte offsets, no raw text leakage.
- Type Design & API: operator names, resource names, and bounded operands are newtypes.
- Performance: summaries are lazy and cached per session; no eager content parsing for profiles that do not reach content-stream properties.
- Documentation: every public feature/report field derived from operators documents whether content bytes are redacted.

## 9. Cross-references

- ← Depends on: [15-parser-filter-source-parity-design.md](./15-parser-filter-source-parity-design.md), [17-validation-model-parity-design.md](./17-validation-model-parity-design.md)
- → Consumed by: [22-resource-font-color-semantics-design.md](./22-resource-font-color-semantics-design.md), [23-structure-accessibility-design.md](./23-structure-accessibility-design.md), [19-verapdf-product-surface-parity-design.md](./19-verapdf-product-surface-parity-design.md)
- ↔ Related review: [../docs/reviews/verapdf-pdfv-core-drift-review.md](../docs/reviews/verapdf-pdfv-core-drift-review.md)
