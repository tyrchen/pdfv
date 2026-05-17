# 17-validation-model-parity-design: Validation Model Parity

Status: draft · Owner: pdfv · Depends on: [13-validation-engine-design.md](./13-validation-engine-design.md), [16-profile-catalog-rule-parity-design.md](./16-profile-catalog-rule-parity-design.md)

## 1. Purpose

This spec expands the validation model graph from the current page/font/annotation/output-intent facts to the broad object surface required by official veraPDF profiles. It owns the registry, property/link naming, lazy materialization, session caches, and conformance-visible feature facts. Detailed semantic subsystems live in [21-content-stream-operator-model-design.md](./21-content-stream-operator-model-design.md), [22-resource-font-color-semantics-design.md](./22-resource-font-color-semantics-design.md), and [23-structure-accessibility-design.md](./23-structure-accessibility-design.md). This spec does not own parser byte syntax, profile XML generation, content operator parsing, deep font/color semantics, accessibility reconstruction, or report writers.

veraPDF's validation-model implementation contains hundreds of wrappers for PD, COS, structure, annotations, actions, colorspaces, images, fonts, functions, signatures, and metadata. The document wrapper alone links pages, metadata, output intents, AcroForm, structure tree root, optional content properties, language, permissions, actions, outlines, and destinations (`vendors/veraPDF-validation/validation-model/src/main/java/org/verapdf/gf/model/impl/pd/GFPDDocument.java:145`). pdfv currently exposes only a narrow subset.

## 2. Interface

The existing `ModelObject` contract remains, but model families are registered in a typed registry:

```rust
pub trait ModelFamily {
    fn family_name(&self) -> ObjectTypeName;
    fn build_roots<'a>(&self, session: &'a ValidationSession) -> Result<Vec<ModelObjectRef<'a>>, PdfvError>;
    fn property_schema(&self) -> &'static [PropertySpec];
    fn link_schema(&self) -> &'static [LinkSpec];
}

pub struct ModelRegistry {
    families: BTreeMap<ObjectTypeName, Arc<dyn ModelFamily + Send + Sync>>,
}
```

The registry is internal to `pdfv-core`. It gives generated profile validation a stable schema to check at build time and keeps the CLI independent of model internals.

## 3. Model Families

Parity is delivered in layers:

| Layer | Families | Required examples |
| --- | --- | --- |
| Document/catalog | document, catalog, metadata, page tree, names, outlines, destinations, language, permissions | Existing document/catalog facts plus AcroForm, structure tree root, OC properties. |
| Page/resources | pages, resources, XObjects, patterns, shadings, color spaces, extGState, properties | Resource inheritance and per-page resource lookup. |
| Fonts/CMaps | Type0, Type1, TrueType, Type3, CIDFont, simple font, embedded font file, ToUnicode, CMap | Embedded status, glyph/code coverage facts, CMap references. |
| Images/content | image XObject, inline image, form XObject, content stream operations, marked content | Filter metadata, dimensions, color space, operator-level facts. |
| Annotations/actions/forms | annotation subtypes, widget/form fields, actions, additional actions, filespecs, rich media, 3D | Forbidden-feature and link traversal rules. |
| Color/transparency | ICCBased, Device*, Cal*, Lab, Indexed, Separation, DeviceN, transparency groups, soft masks | Output-intent and rendering-condition facts. |
| Structure/accessibility | structure tree, struct elements, role maps, artifacts, table/list/heading/link semantics, PDF/UA objects | Tagged PDF and PDF/UA/WTPDF rules. |
| Signatures/security | signature fields, signature dictionaries, DocMDP/FieldMDP, permissions, encryption facts | Validation facts only; cryptographic signature validation is separately scoped. |

The registry lands before the family implementations. Each later subsystem registers its own schema:

- [21-content-stream-operator-model-design.md](./21-content-stream-operator-model-design.md): `ContentStream`, `Operator`, `MarkedContent`, `InlineImage`.
- [22-resource-font-color-semantics-design.md](./22-resource-font-color-semantics-design.md): `Resources`, `Font`, `FontDescriptor`, `CMap`, `ColorSpace`, `ICCProfile`, `OutputIntent`, `XObject`, `ExtGState`, `Pattern`, `Shading`, `Function`.
- [23-structure-accessibility-design.md](./23-structure-accessibility-design.md): `AccessibilityDocument`, `StructureElement`, `TextChunk`, `ImageChunk`, `Artifact`, `Table`, `List`, `Heading`, `Link`, accessibility annotations.

## 4. Property and Link Naming

Property names must match generated profile expectations. The generator in [16-profile-catalog-rule-parity-design.md](./16-profile-catalog-rule-parity-design.md) checks every referenced property/link against the registry.

Rules:

- Direct dictionary properties are available only where the wrapper owns the dictionary.
- Semantic properties are named after veraPDF model properties when compatible.
- Unsupported properties are build-time coverage findings for built-ins and runtime `UnsupportedRule` data for custom profiles.
- Context paths are deterministic and stable enough for snapshots.

## 5. Lazy Materialization and Caches

Model wrappers are lazy. They borrow from `ParsedDocument` and use `ValidationSession` caches for expensive derived data:

- page tree flattening
- inherited resources
- decoded content stream operator summaries
- XMP parse results
- ICC profile summaries
- font/CMap summaries
- structure-tree role maps

Caches are per-session and bounded by `ResourceLimits`. No global or thread-local current document is allowed, preserving [99-key-decisions.md D3](./99-key-decisions.md#d3--replace-threadlocal-state-with-explicit-validationsession).

## 6. Non-goals

- Rendering pages.
- Pixel-perfect image decode unless a conformance rule requires image metadata that cannot be read from headers.
- Full cryptographic signature validation. Signature dictionary facts are in scope; trust-chain validation is not.
- Auto-fixing metadata or documents. That belongs to [19-verapdf-product-surface-parity-design.md](./19-verapdf-product-surface-parity-design.md).

## 7. Invariants

- Every generated built-in rule targets a known object type or is counted unsupported at generation time.
- Every model object has a deterministic identity or a deterministic context path.
- Graph traversal remains iterative and bounded.
- Expensive materialization is cached per session and never shared across inputs.
- Hostile PDFs cannot cause unbounded recursion through page trees, structure trees, resource dictionaries, or annotations.

## 8. AGENTS.md binding

- Error Handling: model materialization failures return `ValidationError` or `ProfileError`, never panic.
- Async & Concurrency: model caches are owned by one `ValidationSession`; no locks are needed for per-file state.
- Type Design & API: object identities, object types, link names, and property names remain newtyped.
- Safety & Security: all graph walks have element/depth caps and cycle detection.
- Testing: fixture families per model layer, regression tests for cycles, property schema tests against generated profiles, and fuzz/corpus tests for hostile graph shapes.
- Performance: materialize only links reached by selected profiles; benches cover page/resource traversal and content summaries.
- Documentation: each model family documents its property/link schema and known unsupported rules.

## 9. Cross-references

- ← Depends on: [13-validation-engine-design.md](./13-validation-engine-design.md), [16-profile-catalog-rule-parity-design.md](./16-profile-catalog-rule-parity-design.md)
- → Consumed by: [18-xmp-metadata-flavour-design.md](./18-xmp-metadata-flavour-design.md), [19-verapdf-product-surface-parity-design.md](./19-verapdf-product-surface-parity-design.md), [21-content-stream-operator-model-design.md](./21-content-stream-operator-model-design.md), [22-resource-font-color-semantics-design.md](./22-resource-font-color-semantics-design.md), [23-structure-accessibility-design.md](./23-structure-accessibility-design.md), [24-parity-metrics-verification-plan.md](./24-parity-metrics-verification-plan.md)
- ↔ Related research/review: [../docs/research/study-verapdf-validator-architecture.md](../docs/research/study-verapdf-validator-architecture.md), [../docs/reviews/verapdf-pdfv-core-drift-review.md](../docs/reviews/verapdf-pdfv-core-drift-review.md)
