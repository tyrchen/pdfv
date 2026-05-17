# 23-structure-accessibility-design: Structure and Accessibility Semantics

Status: draft · Owner: pdfv · Depends on: [17-validation-model-parity-design.md](./17-validation-model-parity-design.md), [21-content-stream-operator-model-design.md](./21-content-stream-operator-model-design.md), [22-resource-font-color-semantics-design.md](./22-resource-font-color-semantics-design.md), [18-xmp-metadata-flavour-design.md](./18-xmp-metadata-flavour-design.md)

## 1. Purpose

This spec treats accessibility/WCAG parity as its own subsystem, not as a small extension to generic model wrappers. It owns structure tree traversal, role/class maps, marked-content association, artifact handling, table/list/heading/link semantics, annotation association, language facts, alt/actual text facts, and PDF/UA/WTPDF rule inputs.

veraPDF has a dedicated WCAG validation module with semantic accessibility objects such as `GFSAPDFDocument`, `GFSAPage`, `GFSAStructElem`, `GFSATextChunk`, `GFSAImageChunk`, tables, lists, annotations, and repeated characters under `vendors/veraPDF-validation/wcag-validation/src/main/java/org/verapdf/gf/model/impl/sa`. The drift review identifies this as product-scope drift.

## 2. Interface

Accessibility reconstruction produces a bounded semantic graph separate from the raw PDF object graph.

```rust
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AccessibilityGraph {
    pub document: AccessibilityNodeId,
    pub nodes: Vec<AccessibilityNode>,
    pub page_associations: Vec<PageContentAssociation>,
    pub warnings: Vec<ValidationWarning>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AccessibilityNode {
    pub id: AccessibilityNodeId,
    pub role: StructureRole,
    pub normalized_role: StructureRole,
    pub source: AccessibilitySource,
    pub attributes: AccessibilityAttributes,
    pub children: Vec<AccessibilityNodeId>,
}
```

The graph is built lazily only when selected profiles or feature extraction need accessibility families.

## 3. Reconstruction Pipeline

1. **Collect document facts**: catalog `/StructTreeRoot`, `/MarkInfo`, `/Lang`, `/ViewerPreferences`, `/AcroForm`, metadata claims from [18-xmp-metadata-flavour-design.md](./18-xmp-metadata-flavour-design.md).
2. **Traverse structure tree**: iterative traversal of `K`, `Pg`, `P`, `/RoleMap`, `/ClassMap`, `/IDTree`, `/ParentTree`, and `/ParentTreeNextKey` with cycle and depth caps.
3. **Normalize roles**: apply `/RoleMap`, validate standard role targets, and record unknown/custom roles.
4. **Associate marked content**: map `MCID` and marked-content property references from [21-content-stream-operator-model-design.md](./21-content-stream-operator-model-design.md) to structure elements.
5. **Associate annotations**: link annotations/widgets to structure elements and pages.
6. **Build semantic families**: document, page, structure element, text chunk, image chunk, annotation, table, list, heading, link, artifact, repeated-character facts.
7. **Emit rule-facing properties**: expose properties used by PDF/UA and WTPDF profiles, with unsupported reasons for missing reconstruction features.

## 4. Semantic Families

| Family | Required facts |
| --- | --- |
| Tagged document | `isTagged`, language, role map presence, mark info, parent tree state. |
| Structure element | role, normalized role, parent/children, page association, alt text, actual text, title, attributes, namespace. |
| Text chunk | marked-content tag, associated structure element, page, text-show presence without default raw text leakage. |
| Image chunk | image XObject/inline image association, alt/actual text facts, artifact status. |
| Annotation/link | annotation subtype, link target/action facts, associated structure element. |
| Table/list/heading | role-derived semantic category, nesting, cell/header relationships where available from attributes. |
| Artifact | artifact type/subtype and page association. |
| Repeated characters | bounded detection from text-show operation patterns; first phase records candidate facts only. |

## 5. Boundaries

In scope:

- PDF/UA and WTPDF validation facts available from structure tree, marked content, annotations, resources, XMP, and page content summaries.
- Feature-report families needed to compare with veraPDF accessibility fixtures.

Out of scope:

- Full assistive-technology reading order simulation.
- OCR, visual layout analysis, or rendered table detection.
- Trusting generated text content by default in reports.
- WCAG policy beyond facts needed by vendored profiles unless separately specified.

## 6. Invariants

- Structure traversal is iterative and bounded by `max_structure_nodes`, `max_structure_depth`, and `max_parent_tree_entries`.
- Role normalization is deterministic and records both source and normalized roles.
- Missing or inconsistent parent-tree/MCID links become facts/warnings; they do not silently drop affected rules.
- Accessibility facts never expose raw page text by default.
- Unsupported accessibility rules are grouped by missing semantic family in parity metrics.

## 7. Tests

- Generated fixtures for tagged document, untagged document, role map, class map, parent tree, marked content association, artifact, link annotation, image alt text, list, table, and malformed cycles.
- PDF/UA and WTPDF profile coverage snapshots before and after accessibility graph rollout.
- Corpus comparison rows against veraPDF for selected accessibility fixtures when licensing permits.
- Fuzz-style tests for cyclic `K` arrays, huge parent trees, invalid role maps, and malformed MCID references.

## 8. AGENTS.md Binding

- Error Handling: malformed structure data is represented as facts/warnings unless traversal cannot continue within limits.
- Safety & Security: every tree/array/dictionary traversal is capped; raw text and alternate text are byte-capped and report-redacted where configured.
- Type Design & API: roles, node ids, MCIDs, and semantic family names are newtypes.
- Performance: graph construction is lazy and cached per validation session.
- Testing: every semantic family requires at least one positive fixture, one malformed fixture, and one unsupported-rule classification test.

## 9. Cross-references

- ← Depends on: [17-validation-model-parity-design.md](./17-validation-model-parity-design.md), [18-xmp-metadata-flavour-design.md](./18-xmp-metadata-flavour-design.md), [21-content-stream-operator-model-design.md](./21-content-stream-operator-model-design.md), [22-resource-font-color-semantics-design.md](./22-resource-font-color-semantics-design.md)
- → Consumed by: [19-verapdf-product-surface-parity-design.md](./19-verapdf-product-surface-parity-design.md), [24-parity-metrics-verification-plan.md](./24-parity-metrics-verification-plan.md)
- ↔ Related review: [../docs/reviews/verapdf-pdfv-core-drift-review.md](../docs/reviews/verapdf-pdfv-core-drift-review.md)
