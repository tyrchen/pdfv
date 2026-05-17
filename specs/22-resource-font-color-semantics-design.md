# 22-resource-font-color-semantics-design: Resource, Font, and Color Semantics

Status: draft · Owner: pdfv · Depends on: [17-validation-model-parity-design.md](./17-validation-model-parity-design.md), [21-content-stream-operator-model-design.md](./21-content-stream-operator-model-design.md)

## 1. Purpose

This spec turns high-risk drift around resources, fonts, color spaces, ICC profiles, images, forms, patterns, shadings, and external graphics state into concrete parity work. It owns semantic summaries needed by PDF/A and PDF/UA rules. It does not own byte parsing, operator tokenization, full font rasterization, image pixel decoding, or visual rendering.

veraPDF has dedicated parser and validation-model packages for resources, fonts, colors, images, patterns, functions, and graphics operators (`vendors/veraPDF-parser/src/main/java/org/verapdf/pd/font`, `vendors/veraPDF-parser/src/main/java/org/verapdf/pd/colors`, `vendors/veraPDF-validation/validation-model/src/main/java/org/verapdf/gf/model/impl/pd/font`, `vendors/veraPDF-validation/validation-model/src/main/java/org/verapdf/gf/model/factory/colors`). The drift review identifies this as one of the main blockers for official-rule parity.

## 2. Interface

Semantic summaries are typed, cached, and report-safe.

```rust
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct EffectiveResources {
    pub owner: ModelObjectId,
    pub fonts: BTreeMap<PdfName, FontSummary>,
    pub color_spaces: BTreeMap<PdfName, ColorSpaceSummary>,
    pub xobjects: BTreeMap<PdfName, XObjectSummary>,
    pub ext_gstates: BTreeMap<PdfName, ExtGStateSummary>,
    pub patterns: BTreeMap<PdfName, PatternSummary>,
    pub shadings: BTreeMap<PdfName, ShadingSummary>,
    pub properties: BTreeMap<PdfName, ObjectKey>,
}
```

`EffectiveResources` is computed for page, form XObject, pattern, and Type 3 font contexts. It applies PDF resource inheritance with cycle detection and object-count caps.

## 3. Resource Semantics

| Area | Concrete solution |
| --- | --- |
| Resource inheritance | Build `ResourceContext` from page tree ancestors and nested form/pattern/type3 contexts. Cache by owner object and inherited parent chain hash. |
| Resource lookup | Normalize all content-stream `ResourceUse` entries into `ResolvedResourceUse { name, family, object, status }`. Missing, wrong-type, and cyclic resources become facts. |
| XObjects | Summarize image/form/PostScript XObjects with subtype, dimensions, filters, resources, group/transparency, and content stream links. |
| External graphics state | Summarize blend mode, alpha constants, overprint, transfer functions, soft masks, and rendering intent names. |
| Patterns/shadings/functions | Summarize type, color space links, resource links, and function object references without evaluating arbitrary mathematical functions unless a rule requires bounded metadata. |

## 4. Font Semantics

| Font family | Required facts |
| --- | --- |
| Type0 | descendant font link, CMap/Encoding name, ToUnicode presence, CIDSystemInfo, embedded descendant program status. |
| Type1/MMType1 | BaseFont, Encoding, FontDescriptor, FontFile presence, widths, symbolic/nonsymbolic flags. |
| TrueType | FontDescriptor, FontFile2 presence, Encoding, ToUnicode, first/last char, widths. |
| Type3 | CharProcs presence, resources, font matrix/bbox, parsed charproc content stream summaries. |
| CIDFontType0/2 | CIDSystemInfo, CIDToGIDMap, embedded font program status, DW/W width facts. |
| CMap | CMapName, CIDSystemInfo, WMode, usecmap reference, embedded stream status. |

Font program parsing is intentionally shallow in the first phase: detect embedded font-file presence, subtype, declared lengths, and identity facts. Full glyph validation requires a later research memo if official rules demand byte-level font-program parsing.

## 5. Color and Output Intent Semantics

| Color object | Required facts |
| --- | --- |
| Output intent | `S`, `DestOutputProfile`, `OutputConditionIdentifier`, `Info`, ICC profile metadata summary. |
| ICCBased | component count, alternate color space, range, metadata, stream filter facts. |
| Device/Cal/Lab/Indexed/Separation/DeviceN | family, base/alternate spaces, tint transform references, component counts. |
| Transparency | transparency group color space, soft-mask facts, blend mode names. |

ICC parsing initially reads only bounded profile header metadata needed for conformance checks: profile size, version, device class, color space, PCS, rendering intent, and tag count. Full ICC tag interpretation is out of scope unless tied to a specific rule cluster.

## 6. Model Schema Additions

Add or expand these model families:

- `Resources`, `Font`, `FontDescriptor`, `FontProgram`, `CMap`
- `ColorSpace`, `ICCProfile`, `OutputIntent`
- `ImageXObject`, `FormXObject`, `PostScriptXObject`
- `ExtGState`, `Pattern`, `Shading`, `Function`
- `ResourceUse`

Each family publishes `PropertySpec` and `LinkSpec` so [16-profile-catalog-rule-parity-design.md](./16-profile-catalog-rule-parity-design.md) can classify unsupported official rules by missing property rather than by opaque evaluation failure.

## 7. Invariants

- Resource inheritance is deterministic and cycle-safe.
- Missing or wrong-type resources are validation facts, not parser panics.
- Font and ICC summaries never trust declared lengths without checking actual stream availability and decode caps.
- Content-stream resource uses resolve through the active `EffectiveResources`, not global document state.
- Feature extraction can include font/color/resource metadata but never raw embedded font bytes or ICC payload bytes.

## 8. Tests

- Resource inheritance fixtures: direct page resources, inherited resources, nested form resources, cycles, missing names, and wrong-type objects.
- Font fixtures: one minimal generated PDF per font family where feasible; otherwise dictionary-level fixtures with explicit unsupported reasons.
- Color fixtures: output intent with ICC header, DeviceRGB/CMYK/Gray, ICCBased, Indexed, Separation, DeviceN, transparency group, soft mask.
- Rule schema tests: official rules referencing font/color/resource properties bind to this spec's schemas or produce named unsupported summaries.
- Benchmarks: resource lookup across 1k pages and repeated content-stream uses.

## 9. AGENTS.md Binding

- Error Handling: malformed resource/font/color summaries return structured validation facts or `ValidationError`; no panics.
- Safety & Security: embedded font and ICC bytes remain hostile; byte caps and checked arithmetic apply to every offset/length.
- Type Design & API: family-specific summary structs use non-exhaustive shapes and newtyped names/ids.
- Performance: resource contexts are cached per session and invalidated only by input boundary; no cross-document cache.
- Documentation: each unsupported font/ICC deep-parse feature must name the rule cluster it blocks.

## 10. Cross-references

- ← Depends on: [17-validation-model-parity-design.md](./17-validation-model-parity-design.md), [21-content-stream-operator-model-design.md](./21-content-stream-operator-model-design.md)
- → Consumed by: [23-structure-accessibility-design.md](./23-structure-accessibility-design.md), [19-verapdf-product-surface-parity-design.md](./19-verapdf-product-surface-parity-design.md)
- ↔ Related review: [../docs/reviews/verapdf-pdfv-core-drift-review.md](../docs/reviews/verapdf-pdfv-core-drift-review.md)
